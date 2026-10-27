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
//!
//! The Cloud-readiness suite's gateway target kind (`gateway`) reaches a
//! gateway by address over the same Connections.

mod cases;
mod connection;
mod fabricated;
mod gateway;
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

use self::cases::{Case, Checks, Site};
use self::connection::{Connection, Gone};
pub use self::fabricated::{FabricatedHelloError, FabricatedHelloOutcome, send_fabricated_hello};
pub use self::gateway::{GatewaySuite, GatewaySuiteError, GatewayTarget, run_gateway_suite};
pub use self::target::{EngineMode, EngineTarget, StartError};
use crate::case_list::{CaseList, DeclaredCapability};
use crate::host::{RUNTIME, platform};
use crate::proving_ground::{ProvingGround, ProvingGroundError};
use crate::writer::{WriteError, write_document};

/// The protocol suite's case list, embedded so the suite runs wherever the
/// harness does.
const CASES: &str = include_str!("../../../conformance/protocol/cases.toml");

/// The page suite's case list (spec #577), run through the same harness:
/// a Collie program's pages over the protocol, on the Proving ground.
const PAGE_CASES: &str = include_str!("../../../conformance/page/cases.toml");

/// The delivery suite's case list, embedded the same way.
const DELIVERY_CASES: &str = include_str!("../../../conformance/delivery/cases.toml");

/// The fetch suite's case list, embedded the same way.
const FETCH_CASES: &str = include_str!("../../../conformance/fetch/cases.toml");

/// The reset gate's case list, embedded the same way.
const RESET_CASES: &str = include_str!("../../../conformance/reset/cases.toml");

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

    /// `conformance/page/cases.toml`.
    pub fn page() -> Self {
        Self::read(PAGE_CASES).expect("invariant: the embedded case list is valid")
    }

    /// `conformance/delivery/cases.toml`: a crawl's work items under their
    /// leases, on Foxhound's executable.
    pub fn delivery() -> Self {
        Self::read(DELIVERY_CASES).expect("invariant: the embedded case list is valid")
    }

    /// `conformance/fetch/cases.toml`: the tier that fetches a crawl
    /// request's document, on Foxhound's executable.
    pub fn fetch() -> Self {
        Self::read(FETCH_CASES).expect("invariant: the embedded case list is valid")
    }

    /// `conformance/reset/cases.toml`: ADR 0001's reset gate, a discarded
    /// Run leaving none of its state or secrets behind.
    pub fn reset() -> Self {
        Self::read(RESET_CASES).expect("invariant: the embedded case list is valid")
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

    /// The suite narrowed to the capabilities `names` names, each with all
    /// its cases, in the suite's own order: a part of a long suite that a
    /// test runs, and times out, on its own. Its document lists those
    /// capabilities alone, under the suite's name and version, so each case
    /// keeps its key in the Feature report.
    ///
    /// # Errors
    ///
    /// When a name is no capability the suite declares.
    pub fn with_capabilities(&self, names: &[&str]) -> Result<Self, SuiteError> {
        if let Some(unknown) = names.iter().find(|name| {
            !self
                .cases
                .capabilities
                .iter()
                .any(|capability| capability.name == **name)
        }) {
            return Err(SuiteError::UnknownCapability {
                suite: self.cases.suite.name.clone(),
                capability: (*unknown).to_owned(),
            });
        }
        Ok(Self {
            product: self.product,
            cases: CaseList {
                suite: self.cases.suite.clone(),
                capabilities: self
                    .cases
                    .capabilities
                    .iter()
                    .filter(|capability| names.contains(&capability.name.as_str()))
                    .cloned()
                    .collect(),
            },
        })
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
    /// A suite was narrowed to a capability it does not declare.
    #[error("the suite `{suite}` declares no capability `{capability}`")]
    UnknownCapability {
        /// The suite's name.
        suite: String,
        /// The name it does not declare.
        capability: String,
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

/// The variable of the target's environment that holds the password of the
/// Proving ground's proxies, when it serves them: the harness sets it, so a
/// case's Session names it through the `env:` Secret provider. It is no
/// `GOODFETCH_` variable, a name the engine refuses unless it deploys it.
pub const PROXY_PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

/// Runs `suite` against `target` and the running `proving_ground`, and
/// returns its result document. The target starts with
/// [`PROXY_PASSWORD_VARIABLE`] added to its environment when the Proving
/// ground's proxies take credentials.
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
    let reached = Site::of(&task_list, proving_ground);
    // The harness gives the target the password of the Proving ground's
    // proxies, which a case's Session reads through the `env:` Secret
    // provider, as a program's own environment would hold it.
    let mut given = target.clone();
    if let Some(credentials) = reached.proxy_credentials() {
        given.environment.insert(
            PROXY_PASSWORD_VARIABLE.to_owned(),
            credentials.password.clone(),
        );
    }
    let target = &given;
    let mut connection = Connection::new(target::Started::start(target)?);
    let hello = match hello(&mut connection, suite.product) {
        Ok(hello) => hello,
        Err(refused) => {
            if matches!(refused, ProtocolSuiteError::OtherProduct { .. }) {
                stop(&mut connection, target.mode);
            }
            connection.close();
            return Err(refused);
        }
    };
    let opening = connection.take_exchanged();
    let mut log = Vec::new();
    let mut gone = None;
    for capability in &suite.cases.capabilities {
        for declared in &capability.cases {
            if gone.is_some() {
                break;
            }
            let case = Case::of(&declared.id).expect("invariant: a read suite names known cases");
            let mut checks = Checks::default();
            if let Err(why) = case.run(&mut connection, target, &hello, &reached, &mut checks) {
                checks.checks.push(CaseCheck {
                    name: format!("the target answers the case's calls ({why})"),
                    passed: false,
                });
                gone = Some(why);
            }
            let mut messages = connection.take_exchanged();
            if case == Case::Hello {
                messages.splice(0..0, opening.iter().cloned());
            }
            log.push(CaseLogEntry {
                case: declared.id.clone(),
                capability: capability.name.clone(),
                mode: Some(target.mode.as_str().to_owned()),
                platform: platform(),
                runtime: RUNTIME.to_owned(),
                checks: checks.checks,
                evidence: Evidence {
                    messages,
                    oracle_answers: checks.oracle_answers,
                    ..Evidence::default()
                },
            });
        }
    }
    // A Connection that only fell silent may still take the stop.
    if matches!(gone, None | Some(Gone::Silent(_))) {
        stop(&mut connection, target.mode);
    }
    connection.close();
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
fn stop(connection: &mut Connection, mode: EngineMode) {
    if mode == EngineMode::Daemon
        && let Ok(id) = connection.request(STOP.name, &json!({}))
    {
        let _ = connection.answer(id);
    }
}

/// Opens the Connection with `hello`, announcing `product`, and checks that
/// the engine runs as that Product.
fn hello(connection: &mut Connection, product: Product) -> Result<HelloResult, ProtocolSuiteError> {
    hello_as(connection, product, Surface::Rust)
}

/// Opens the Connection with `hello`, announcing `product` from a Client of
/// `surface`, such as an Agent's through the MCP server, and checks that
/// the engine runs as that Product.
pub(crate) fn hello_as(
    connection: &mut Connection,
    product: Product,
    surface: Surface,
) -> Result<HelloResult, ProtocolSuiteError> {
    let params = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface,
        form: None,
        token: connection.token().map(ToOwned::to_owned),
        needs: Some(product),
        product_version: ProductVersion::parse(env!("CARGO_PKG_VERSION"))
            .expect("invariant: the workspace's version is a Product version"),
        log_level: None,
    };
    let params = serde_json::to_value(params).expect("invariant: `hello` serializes");
    let unanswered = |why: Gone| ProtocolSuiteError::HelloUnanswered {
        reason: why.to_string(),
    };
    let id = connection.request(HELLO, &params).map_err(unanswered)?;
    connection.set_for_model(surface.acts_for_model());
    let answer = connection.first_answer(id).map_err(unanswered)?.message;
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
