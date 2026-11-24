//! The process Plugin target (#171): a Plugin of any kind, started from its
//! executable and arguments, spoken to over the plugin protocol on its stdio
//! (ADR 0033), so each Plugin kind's suite proves the capabilities the
//! Plugin declares one at a time.
//!
//! The harness stands for the engine: it starts the Plugin with the
//! protocol's cookie, sends `hello` announcing the suite's Product, and
//! reads the kinds and capabilities the Plugin declares. A Plugin that
//! declares no implementation of the suite's kind is refused before any
//! case. Every case of each capability the Plugin declares then runs in
//! the suite's order over the one Connection, and each case's checks and
//! the messages it exchanged become a case log entry; a capability it does
//! not declare is never reached, nor is any case once the Plugin closed the
//! Connection, which leaves their capabilities `unproved`.

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::{
    DeclaredKind, HELLO, PLUGIN_PROTOCOL_VERSION, PluginHelloParams, PluginHelloResult,
};
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, Target, TargetIdentity,
};
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

/// The `sample_store` kind's suite, embedded so it runs wherever the harness
/// does.
const SAMPLE_STORE_CASES: &str = include_str!("../../../../conformance/sample-store/cases.toml");

/// A process Plugin: an executable taken by path, started with these
/// arguments and exactly this environment, to which the harness adds the
/// plugin protocol's cookie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginTarget {
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

    /// Reads a Plugin kind's case list.
    ///
    /// # Errors
    ///
    /// When the text is no case list, or a case ID names no exchange the
    /// harness runs.
    pub fn read(text: &str) -> Result<Self, SuiteError> {
        let read: PluginCases = toml::from_str(text).map_err(SuiteError::Malformed)?;
        if let Some(case) = read
            .capabilities
            .iter()
            .flat_map(|capability| &capability.cases)
            .find(|case| Case::of(&case.id).is_none())
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
/// When the Plugin cannot be started, does not answer `hello` with a
/// `hello` result, or declares no implementation of the suite's kind, and
/// when the Proving ground cannot serve its task list.
pub fn run_plugin_suite(
    suite: &PluginSuite,
    target: &PluginTarget,
    proving_ground: &ProvingGround,
) -> Result<ConformanceDocument, PluginSuiteError> {
    let task_list = proving_ground.task_list()?;
    let mut connection = Connection::to_plugin(Started::plugin(target)?);
    let (hello, declared) = match hello(&mut connection, suite) {
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
            let case =
                Case::of(&declared_case.id).expect("invariant: a read suite names known cases");
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

/// Opens the plugin protocol with `hello`, announcing the suite's Product,
/// and finds the Plugin's declaration of the suite's kind.
fn hello(
    connection: &mut Connection,
    suite: &PluginSuite,
) -> Result<(PluginHelloResult, DeclaredKind), PluginSuiteError> {
    let params = serde_json::to_value(PluginHelloParams {
        protocol_version: PLUGIN_PROTOCOL_VERSION,
        product: suite.product,
    })
    .expect("invariant: `hello` serializes");
    let unanswered = |why: Gone| PluginSuiteError::HelloUnanswered {
        reason: why.to_string(),
    };
    let id = connection.request(HELLO, &params).map_err(unanswered)?;
    let answer = connection.first_answer(id).map_err(unanswered)?.message;
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
    let Some(declared) = hello
        .kinds
        .iter()
        .find(|declared| declared.kind == suite.kind)
        .cloned()
    else {
        return Err(PluginSuiteError::KindUndeclared {
            kind: suite.kind.clone(),
            declared: hello.kinds.iter().map(|kind| kind.kind.clone()).collect(),
        });
    };
    Ok((hello, declared))
}

/// The method of the `sample_store` kind that keeps a value.
const SAMPLE_STORE_PUT: &str = "sample_store.put";

/// An exchange a Plugin kind's case runs, named by its case ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    /// `sample_store.put` under a key never used replaces nothing.
    NewKey,
    /// A second `sample_store.put` under a key replaces the first value.
    KeptKey,
    /// `sample_store.put` with a field a newer engine adds is answered as
    /// without it.
    NewerField,
}

impl Case {
    /// The exchange `id` names.
    fn of(id: &str) -> Option<Self> {
        match id {
            "a-value-under-a-new-key-replaces-nothing" => Some(Self::NewKey),
            "a-value-under-a-kept-key-replaces-it" => Some(Self::KeptKey),
            "params-from-a-newer-engine-are-read-without-their-unknown-field" => {
                Some(Self::NewerField)
            }
            _ => None,
        }
    }

    /// Runs the exchange, adding its checks to `checks`.
    ///
    /// # Errors
    ///
    /// Why the Connection is gone, after which no case runs.
    fn run(self, connection: &mut Connection, checks: &mut Vec<CaseCheck>) -> Result<(), Gone> {
        match self {
            Self::NewKey => {
                let answer = put(connection, &json!({ "key": "new", "value": "1" }))?;
                checks.push(replaced(
                    "the value under a new key replaces nothing",
                    &answer,
                    false,
                ));
            }
            Self::KeptKey => {
                let first = put(connection, &json!({ "key": "kept", "value": "1" }))?;
                checks.push(replaced(
                    "the first value under the key replaces nothing",
                    &first,
                    false,
                ));
                let second = put(connection, &json!({ "key": "kept", "value": "2" }))?;
                checks.push(replaced(
                    "the second value under the key replaces the first",
                    &second,
                    true,
                ));
            }
            Self::NewerField => {
                let answer = put(
                    connection,
                    &json!({ "key": "newer", "value": "1", "added_in_a_minor": 1 }),
                )?;
                checks.push(replaced(
                    "params with a field the Plugin does not know are answered as without it",
                    &answer,
                    false,
                ));
            }
        }
        Ok(())
    }
}

/// Sends `sample_store.put` with `params` and reads its answer.
fn put(connection: &mut Connection, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(SAMPLE_STORE_PUT, params)?;
    Ok(connection.answer(id)?.message)
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
