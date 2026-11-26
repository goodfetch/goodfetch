//! The process Plugin target (#171): a Plugin of any kind, started from its
//! executable and arguments, spoken to over the plugin protocol on its stdio
//! (ADR 0033), so each Plugin kind's suite proves the capabilities the
//! Plugin declares one at a time.
//!
//! The harness stands for the engine: it starts the Plugin with the
//! protocol's cookie, sends `hello` with the plugin protocol range it
//! speaks, the suite's Product, a run ID and the approved manifest's network
//! level, and reads the kinds and capabilities the Plugin declares. The
//! answer must be the Plugin's first message, and is held to the approved
//! manifest as the engine holds it (ADR 0066): a refusal, a `hello` that
//! differs from the manifest, and a Plugin that declares no implementation
//! of the suite's kind are refused before any case. Every case of each
//! capability the Plugin declares then runs in
//! the suite's order over the one Connection, and each case's checks and
//! the messages it exchanged become a case log entry; a capability it does
//! not declare is never reached, nor is any case once the Plugin closed the
//! Connection, which leaves their capabilities `unproved`.

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::{
    DeclaredKind, HELLO, HelloRefusal, PLUGIN_PROTOCOL, PluginHelloParams, PluginHelloResult,
    PluginManifest, check_hello,
};
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, Target, TargetIdentity,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::sample::SampleStorePutResult;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::SuiteError;
use super::connection::{Connection, Gone};
use super::target::{StartError, Started};
use crate::case_list::{CaseList, DeclaredCapability};
use crate::host::{RUNTIME, platform};
use crate::proving_ground::{ProvingGround, ProvingGroundError};
use crate::writer::{WriteError, write_document};

mod extract;

/// The `sample_store` kind's suite, embedded so it runs wherever the harness
/// does.
const SAMPLE_STORE_CASES: &str = include_str!("../../../../conformance/sample-store/cases.toml");

/// The Extractor's suite, embedded the same way.
const EXTRACT_CASES: &str = include_str!("../../../../conformance/extract/cases.toml");

/// The run ID the harness's `hello` names.
const RUN_ID: &str = "conformance";

/// A process Plugin: an executable taken by path, started with these
/// arguments and exactly this environment, to which the harness adds the
/// plugin protocol's cookie, and held to its approved manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginTarget {
    /// The manifest approved for it, which its `hello` must match.
    pub manifest: PluginManifest,
    /// The executable.
    pub executable: PathBuf,
    /// Its arguments.
    pub arguments: Vec<String>,
    /// The environment it runs with.
    pub environment: BTreeMap<String, String>,
    /// The folder it runs in.
    pub folder: PathBuf,
}

/// A Plugin kind's suite: its declared case list, the kind it proves, and
/// the Product whose executable runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSuite {
    product: Product,
    kind: String,
    cases: CaseList,
}

/// A Plugin kind's case list as its file writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginCases {
    product: Product,
    kind: String,
    suite: goodfetch_protocol::result_document::Suite,
    #[serde(rename = "capability")]
    capabilities: Vec<DeclaredCapability>,
}

impl PluginSuite {
    /// `conformance/sample-store/cases.toml`: Collie's sample Plugin kind.
    pub fn sample_store() -> Self {
        Self::read(SAMPLE_STORE_CASES).expect("invariant: the embedded case list is valid")
    }

    /// `conformance/extract/cases.toml`: the Extractor's.
    pub fn extract() -> Self {
        Self::read(EXTRACT_CASES).expect("invariant: the embedded case list is valid")
    }

    /// Reads a Plugin kind's case list.
    ///
    /// # Errors
    ///
    /// When the text is no case list, or a case ID names no exchange the
    /// harness runs for its kind.
    pub fn read(text: &str) -> Result<Self, SuiteError> {
        let read: PluginCases = toml::from_str(text).map_err(SuiteError::Malformed)?;
        if let Some(case) = read
            .capabilities
            .iter()
            .flat_map(|capability| &capability.cases)
            .find(|case| Case::of(&read.kind, &case.id).is_none())
        {
            return Err(SuiteError::UnknownCase {
                case: case.id.clone(),
            });
        }
        Ok(Self {
            product: read.product,
            kind: read.kind,
            cases: CaseList {
                suite: read.suite,
                capabilities: read.capabilities,
            },
        })
    }

    /// The Plugin kind it proves.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The Product whose executable runs it, which its `hello` announces.
    pub fn product(&self) -> Product {
        self.product
    }

    /// Its declared case list.
    pub fn case_list(&self) -> &CaseList {
        &self.cases
    }

    /// The checks of each of its cases, in its order, run against an
    /// implementation of its kind called in process, which `answer` stands
    /// for: given a method and its params, it answers what a process
    /// Plugin's response would hold, `{ "result": … }` or
    /// `{ "error": … }`. Every capability is reached: an implementation
    /// in process fills the whole kind. So the engine's own
    /// implementations pass the same cases as a Plugin (ADR 0129).
    pub fn checks_in_process(
        &self,
        answer: impl FnMut(&str, &Value) -> Value,
    ) -> Vec<(String, Vec<CaseCheck>)> {
        let mut calls = InProcess(answer);
        self.cases
            .capabilities
            .iter()
            .flat_map(|capability| &capability.cases)
            .map(|declared| {
                let case = Case::of(&self.kind, &declared.id)
                    .expect("invariant: a read suite names known cases");
                let mut checks = Vec::new();
                case.run(&mut calls, &mut checks)
                    .expect("invariant: a call in process always answers");
                (declared.id.clone(), checks)
            })
            .collect()
    }
}

/// What a case calls: an implementation of the suite's kind, which answers
/// each call with its response, or is gone.
trait Calls {
    /// The response to `method` with `params`.
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone>;
}

impl Calls for Connection {
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        let id = self.request(method, params)?;
        Ok(self.answer(id)?.message)
    }
}

/// An implementation called in process, whose answer is the response.
struct InProcess<F>(F);

impl<F: FnMut(&str, &Value) -> Value> Calls for InProcess<F> {
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        Ok((self.0)(method, params))
    }
}

/// Why a Plugin kind's suite could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginSuiteError {
    /// The Plugin could not be started.
    #[error(transparent)]
    Start(#[from] StartError),
    /// The Plugin gave no answer to `hello`.
    #[error("the Plugin answered no `hello`: {reason}")]
    HelloUnanswered {
        /// Why.
        reason: String,
    },
    /// The Plugin sent another message before its answer to `hello`.
    #[error("the Plugin sent {first} before its answer to `hello`")]
    HelloNotFirst {
        /// The first message it sent.
        first: Value,
    },
    /// The Plugin refused the `hello`.
    #[error("the Plugin refused `hello`: {error}")]
    HelloRefused {
        /// The error object it answered with.
        error: Value,
    },
    /// The Plugin's answer to `hello` is no plugin protocol `hello` result.
    #[error("the Plugin's answer to `hello` is no `hello` result: {answer}")]
    HelloUnreadable {
        /// The answer.
        answer: Value,
    },
    /// The Plugin's `hello` result is refused as the engine refuses it:
    /// a version the harness does not speak, or a claim its approved
    /// manifest does not make.
    #[error(transparent)]
    HelloNotAccepted(#[from] HelloRefusal),
    /// The Plugin declares no implementation of the suite's kind.
    #[error("the suite proves the Plugin kind `{kind}`, and the Plugin declares only {declared:?}")]
    KindUndeclared {
        /// The suite's kind.
        kind: String,
        /// The kinds the Plugin declares.
        declared: Vec<String>,
    },
    /// The Proving ground gave no usable answer.
    #[error(transparent)]
    ProvingGround(#[from] ProvingGroundError),
    /// The case log did not become a document.
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// Runs `suite` against the process Plugin `target` and the running
/// `proving_ground`, and returns its result document.
///
/// # Errors
///
/// When the Plugin cannot be started, does not answer `hello` first with a
/// `hello` result its approved manifest accepts, or declares no
/// implementation of the suite's kind, and when the Proving ground cannot
/// serve its task list.
pub fn run_plugin_suite(
    suite: &PluginSuite,
    target: &PluginTarget,
    proving_ground: &ProvingGround,
) -> Result<ConformanceDocument, PluginSuiteError> {
    let task_list = proving_ground.task_list()?;
    let mut connection = Connection::to_plugin(Started::plugin(target)?);
    let (hello, declared) = match hello(&mut connection, suite, &target.manifest) {
        Ok(answered) => answered,
        Err(refused) => {
            connection.close();
            return Err(refused);
        }
    };
    let mut opening = connection.take_exchanged();
    let mut log = Vec::new();
    let mut gone = None;
    for capability in &suite.cases.capabilities {
        if !declared.capabilities.contains(&capability.name) {
            continue;
        }
        for declared_case in &capability.cases {
            if gone.is_some() {
                break;
            }
            let case = Case::of(&suite.kind, &declared_case.id)
                .expect("invariant: a read suite names known cases");
            let mut checks = Vec::new();
            if let Err(why) = case.run(&mut connection, &mut checks) {
                checks.push(CaseCheck {
                    name: format!("the Plugin answers the case's calls ({why})"),
                    passed: false,
                });
                gone = Some(why);
            }
            let mut messages = std::mem::take(&mut opening);
            messages.extend(connection.take_exchanged());
            log.push(CaseLogEntry {
                case: declared_case.id.clone(),
                capability: capability.name.clone(),
                mode: None,
                blocking: None,
                unreachable: Vec::new(),
                platform: platform(),
                runtime: RUNTIME.to_owned(),
                checks,
                evidence: Evidence {
                    messages,
                    ..Evidence::default()
                },
            });
        }
    }
    connection.close();
    let header = ConformanceHeader {
        suite: suite.cases.suite.clone(),
        target: Target {
            name: hello.name,
            identity: TargetIdentity::Version(hello.version),
            product: None,
        },
        platform: platform(),
        runtime: RUNTIME.to_owned(),
        proving_ground: task_list.proving_ground,
        online: proving_ground.is_online(),
        gate: false,
    };
    Ok(write_document(header, &suite.cases, &log)?)
}

/// Opens the plugin protocol with `hello`, announcing the suite's Product
/// and the approved manifest's network level, holds the answer, which must
/// come first, to `approved`, and finds the Plugin's declaration of the
/// suite's kind.
fn hello(
    connection: &mut Connection,
    suite: &PluginSuite,
    approved: &PluginManifest,
) -> Result<(PluginHelloResult, DeclaredKind), PluginSuiteError> {
    let params = serde_json::to_value(PluginHelloParams {
        protocol: PLUGIN_PROTOCOL,
        product: suite.product,
        run_id: Some(RunId::parse(RUN_ID).expect("invariant: the harness's run ID is a Run ID")),
        network_level: approved.network_level,
    })
    .expect("invariant: `hello` serializes");
    let unanswered = |why: Gone| PluginSuiteError::HelloUnanswered {
        reason: why.to_string(),
    };
    let id = connection.request(HELLO, &params).map_err(unanswered)?;
    let answered = connection.first_answer(id).map_err(unanswered)?;
    if let Some(first) = answered.others.into_iter().next() {
        return Err(PluginSuiteError::HelloNotFirst { first });
    }
    let answer = answered.message;
    if let Some(error) = answer.get("error") {
        return Err(PluginSuiteError::HelloRefused {
            error: error.clone(),
        });
    }
    let hello: PluginHelloResult = answer
        .get("result")
        .and_then(|result| serde_json::from_value(result.clone()).ok())
        .ok_or_else(|| PluginSuiteError::HelloUnreadable {
            answer: answer.clone(),
        })?;
    check_hello(PLUGIN_PROTOCOL, approved, &hello)?;
    let Some(declared) = hello
        .kinds
        .iter()
        .find(|declared| declared.kind.as_str() == suite.kind)
        .cloned()
    else {
        return Err(PluginSuiteError::KindUndeclared {
            kind: suite.kind.clone(),
            declared: hello
                .kinds
                .iter()
                .map(|kind| kind.kind.as_str().to_owned())
                .collect(),
        });
    };
    Ok((hello, declared))
}

/// The method of the `sample_store` kind that keeps a value.
const SAMPLE_STORE_PUT: &str = "sample_store.put";

/// An exchange a Plugin kind's case runs, named by its kind and case ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    /// `sample_store.put` under a key never used replaces nothing.
    NewKey,
    /// A second `sample_store.put` under a key replaces the first value.
    KeptKey,
    /// `sample_store.put` with a field a newer engine adds is answered as
    /// without it.
    NewerField,
    /// `extract.content` of the article page answers its main content in
    /// the kind's shape.
    ExtractArticle,
    /// `extract.content` with a field a newer engine adds is answered as
    /// without it.
    ExtractNewerField,
}

/// The ID of each kind's case of params from a newer engine.
const NEWER_PARAMS: &str = "params-from-a-newer-engine-are-read-without-their-unknown-field";

impl Case {
    /// The exchange `id` names in the suite of `kind`.
    fn of(kind: &str, id: &str) -> Option<Self> {
        match (kind, id) {
            ("sample_store", "a-value-under-a-new-key-replaces-nothing") => Some(Self::NewKey),
            ("sample_store", "a-value-under-a-kept-key-replaces-it") => Some(Self::KeptKey),
            ("sample_store", NEWER_PARAMS) => Some(Self::NewerField),
            ("extract", "a-pages-main-content-is-answered-in-the-kinds-shape") => {
                Some(Self::ExtractArticle)
            }
            ("extract", NEWER_PARAMS) => Some(Self::ExtractNewerField),
            _ => None,
        }
    }

    /// Runs the exchange against `calls`, adding its checks to `checks`.
    ///
    /// # Errors
    ///
    /// Why the Connection is gone, after which no case runs.
    fn run(self, calls: &mut dyn Calls, checks: &mut Vec<CaseCheck>) -> Result<(), Gone> {
        let mut put = |params: &Value| calls.call(SAMPLE_STORE_PUT, params);
        match self {
            Self::NewKey => {
                let answer = put(&json!({ "key": "new", "value": "1" }))?;
                checks.push(replaced(
                    "the value under a new key replaces nothing",
                    &answer,
                    false,
                ));
            }
            Self::KeptKey => {
                let first = put(&json!({ "key": "kept", "value": "1" }))?;
                checks.push(replaced(
                    "the first value under the key replaces nothing",
                    &first,
                    false,
                ));
                let second = put(&json!({ "key": "kept", "value": "2" }))?;
                checks.push(replaced(
                    "the second value under the key replaces the first",
                    &second,
                    true,
                ));
            }
            Self::NewerField => {
                let answer = put(&json!({ "key": "newer", "value": "1", "added_in_a_minor": 1 }))?;
                checks.push(replaced(
                    "params with a field the Plugin does not know are answered as without it",
                    &answer,
                    false,
                ));
            }
            Self::ExtractArticle | Self::ExtractNewerField => {
                let newer = self == Self::ExtractNewerField;
                let answer = calls.call(extract::METHOD, &extract::params(newer))?;
                checks.extend(extract::checks(&answer));
            }
        }
        Ok(())
    }
}

/// The check `name`: `answer` is a `sample_store.put` result whose
/// `replaced` is `expected`.
fn replaced(name: &str, answer: &Value, expected: bool) -> CaseCheck {
    let result = answer
        .get("result")
        .and_then(|result| SampleStorePutResult::deserialize(result).ok());
    CaseCheck {
        name: name.to_owned(),
        passed: result.is_some_and(|result| result.replaced == expected),
    }
}
