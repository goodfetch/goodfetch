//! The JSON-RPC protocol's target (#172): one adapter for every target that
//! speaks Goodfetch's protocol, either Product's Attached engine or Daemon
//! (its executable taken by path), the Browser host and Goodfetch Cloud's
//! engine, which is a published Product executable run unmodified (ADRs
//! 0001, 0022 and 0096), so one suite covers local and cloud behaviour.
//!
//! The harness opens one Connection with `hello`, announcing the Product
//! the suite tests (ADR 0095), and runs every declared case over it in
//! order. A suite runs on the executable of the smallest Product that
//! carries its contract (ADR 0094), so an engine of another Product than
//! the suite's is refused before any case. Each case's checks and the
//! messages it exchanged become a case log entry, and the writer turns the
//! log into the result document: a case after the target closed the
//! Connection is not reached, which leaves its capability `unproved`.

mod cases;
mod session;
mod target;

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult, ProductVersion, Surface};
use goodfetch_protocol::daemon::STOP;
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, Suite, Target, TargetIdentity,
};
use serde::Deserialize;
use serde_json::{Value, json};

use self::cases::{Case, Checks};
use self::session::{Gone, Session};
pub use self::target::{EngineMode, EngineTarget, StartError};
use crate::case_list::{CaseList, DeclaredCapability};
use crate::host::{RUNTIME, platform};
use crate::proving_ground::{ProvingGround, ProvingGroundError};
use crate::writer::{WriteError, write_document};

/// The protocol suite's case list, embedded so the suite runs wherever the
/// harness does.
const CASES: &str = include_str!("../../../conformance/protocol/cases.toml");

/// A protocol suite: its declared case list, and the Product whose
/// executable it runs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolSuite {
    product: Product,
    cases: CaseList,
}

/// A protocol suite's case list as its file writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolCases {
    product: Product,
    suite: Suite,
    #[serde(rename = "capability")]
    capabilities: Vec<DeclaredCapability>,
}

impl ProtocolSuite {
    /// `conformance/protocol/cases.toml`.
    pub fn embedded() -> Self {
        Self::read(CASES).expect("invariant: the embedded case list is valid")
    }

    /// Reads a protocol suite's case list.
    ///
    /// # Errors
    ///
    /// When the text is no case list, or a case ID names no exchange the
    /// harness runs.
    pub fn read(text: &str) -> Result<Self, SuiteError> {
        let read: ProtocolCases = toml::from_str(text).map_err(SuiteError::Malformed)?;
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
            cases: CaseList {
                suite: read.suite,
                capabilities: read.capabilities,
            },
        })
    }

    /// The Product it tests, which its `hello` announces.
    pub fn product(&self) -> Product {
        self.product
    }

    /// Its declared case list.
    pub fn case_list(&self) -> &CaseList {
        &self.cases
    }
}

/// Why a protocol suite's case list was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SuiteError {
    /// The text is no case list.
    #[error("the text is no protocol suite's case list: {0}")]
    Malformed(toml::de::Error),
    /// A case ID names no exchange the harness runs.
    #[error("the case `{case}` names no exchange the harness runs")]
    UnknownCase {
        /// Its ID.
        case: String,
    },
}

/// Why a protocol suite could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProtocolSuiteError {
    /// The target could not be reached.
    #[error(transparent)]
    Start(#[from] StartError),
    /// The target gave no answer to `hello`.
    #[error("the target answered no `hello`: {reason}")]
    HelloUnanswered {
        /// Why.
        reason: String,
    },
    /// The target refused the `hello`, such as with `not_in_product` for a
    /// suite of a Product it does not contain.
    #[error("the target refused `hello`: {error}")]
    HelloRefused {
        /// The error object it answered with.
        error: Value,
    },
    /// The target's answer to `hello` is no `hello` result.
    #[error("the target's answer to `hello` is no `hello` result: {answer}")]
    HelloUnreadable {
        /// The answer.
        answer: Value,
    },
    /// The target runs as another Product than the suite's, whose
    /// executable the suite runs on (ADR 0094).
    #[error(
        "the suite tests {suite:?}, so it runs on {suite:?}'s executable, and the target runs as {engine:?}"
    )]
    OtherProduct {
        /// The suite's Product.
        suite: Product,
        /// The engine's.
        engine: Product,
    },
    /// The Proving ground gave no usable answer.
    #[error(transparent)]
    ProvingGround(#[from] ProvingGroundError),
    /// The case log did not become a document.
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// Runs `suite` against `target` and the running `proving_ground`, and
/// returns its result document.
///
/// # Errors
///
/// When the target cannot be reached, refuses or misreads `hello`, or runs
/// as another Product than the suite's, and when the Proving ground cannot
/// serve its task list.
pub fn run_protocol_suite(
    suite: &ProtocolSuite,
    target: &EngineTarget,
    proving_ground: &ProvingGround,
) -> Result<ConformanceDocument, ProtocolSuiteError> {
    let task_list = proving_ground.task_list()?;
    let mut session = Session::new(target::Started::start(target)?);
    let hello = match hello(&mut session, suite.product) {
        Ok(hello) => hello,
        Err(refused) => {
            if matches!(refused, ProtocolSuiteError::OtherProduct { .. }) {
                stop(&mut session, target.mode);
            }
            session.close();
            return Err(refused);
        }
    };
    let opening = session.take_exchanged();
    let mut log = Vec::new();
    let mut gone = None;
    for capability in &suite.cases.capabilities {
        for declared in &capability.cases {
            if gone.is_some() {
                break;
            }
            let case = Case::of(&declared.id).expect("invariant: a read suite names known cases");
            let mut checks = Checks::default();
            if let Err(why) = case.run(&mut session, &hello, &mut checks) {
                checks.0.push(CaseCheck {
                    name: format!("the target answers the case's calls ({why})"),
                    passed: false,
                });
                gone = Some(why);
            }
            let mut messages = session.take_exchanged();
            if case == Case::Hello {
                messages.splice(0..0, opening.iter().cloned());
            }
            log.push(CaseLogEntry {
                case: declared.id.clone(),
                capability: capability.name.clone(),
                mode: Some(target.mode.as_str().to_owned()),
                platform: platform(),
                runtime: RUNTIME.to_owned(),
                checks: checks.0,
                evidence: Evidence {
                    messages,
                    ..Evidence::default()
                },
            });
        }
    }
    // A Connection that only fell silent may still take the stop.
    if matches!(gone, None | Some(Gone::Silent)) {
        stop(&mut session, target.mode);
    }
    session.close();
    let header = ConformanceHeader {
        suite: suite.cases.suite.clone(),
        target: Target {
            name: target
                .executable
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            identity: TargetIdentity::Version(hello.product_version.to_string()),
            product: Some(hello.product),
        },
        platform: platform(),
        runtime: RUNTIME.to_owned(),
        proving_ground: task_list.proving_ground,
        online: proving_ground.is_online(),
    };
    Ok(write_document(header, &suite.cases, &log)?)
}

/// Stops a Daemon the harness started, over its open Connection, so it
/// removes its runtime file and socket; one that cannot answer is stopped
/// when the harness closes.
fn stop(session: &mut Session, mode: EngineMode) {
    if mode == EngineMode::Daemon
        && let Ok(id) = session.request(STOP.name, &json!({}))
    {
        let _ = session.answer(id);
    }
}

/// Opens the Connection with `hello`, announcing `product`, and checks that
/// the engine runs as that Product.
fn hello(session: &mut Session, product: Product) -> Result<HelloResult, ProtocolSuiteError> {
    let params = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Rust,
        token: session.token().map(ToOwned::to_owned),
        needs: Some(product),
        product_version: ProductVersion::parse(env!("CARGO_PKG_VERSION"))
            .expect("invariant: the workspace's version is a Product version"),
    };
    let params = serde_json::to_value(params).expect("invariant: `hello` serializes");
    let unanswered = |why: Gone| ProtocolSuiteError::HelloUnanswered {
        reason: why.to_string(),
    };
    let id = session.request(HELLO, &params).map_err(unanswered)?;
    let answer = session.answer(id).map_err(unanswered)?.message;
    if let Some(error) = answer.get("error") {
        return Err(ProtocolSuiteError::HelloRefused {
            error: error.clone(),
        });
    }
    let hello: HelloResult = answer
        .get("result")
        .and_then(|result| serde_json::from_value(result.clone()).ok())
        .ok_or_else(|| ProtocolSuiteError::HelloUnreadable {
            answer: answer.clone(),
        })?;
    if hello.product != product {
        return Err(ProtocolSuiteError::OtherProduct {
            suite: product,
            engine: hello.product,
        });
    }
    Ok(hello)
}
