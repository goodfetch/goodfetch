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
mod foxhound;
mod gateway;
mod target;

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult, Surface};
use goodfetch_protocol::daemon::STOP;
use goodfetch_protocol::result_document::{
    Blocking, CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, Suite, Target,
    TargetIdentity,
};
use serde::Deserialize;
use serde_json::{Value, json};

use self::cases::{Case, Checks, Site};
use self::connection::{Connection, Gone};
pub use self::fabricated::{FabricatedHelloError, FabricatedHelloOutcome, send_fabricated_hello};
pub use self::gateway::{
    GatewayAddress, GatewayAddressError, GatewaySuite, GatewaySuiteError, GatewayTarget,
    run_gateway_suite,
};
pub use self::target::{EngineMode, EngineTarget, StartError};
use crate::case_list::{CaseList, DeclaredCapability, DeclaredCase};
use crate::host::{RUNTIME, platform};
use crate::proving_ground::{ProvingGround, ProvingGroundError, TaskList};
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

/// The start table's case list, embedded the same way.
const RUN_START_CASES: &str = include_str!("../../../conformance/run-start/cases.toml");

/// The start table's crawl rows, embedded the same way.
const RUN_START_CRAWL_CASES: &str = include_str!("../../../conformance/run-start-crawl/cases.toml");

/// The two-copies suite's case list, embedded the same way.
const TWO_COPIES_CASES: &str = include_str!("../../../conformance/two-copies/cases.toml");

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

    /// `conformance/run-start/cases.toml`: ADR 0102's start table, a start
    /// under a Run ID reaching the Run it names.
    pub fn run_start() -> Self {
        Self::read(RUN_START_CASES).expect("invariant: the embedded case list is valid")
    }

    /// `conformance/run-start-crawl/cases.toml`: ADR 0102's start table
    /// rows a crawl reaches, `run_conflict` and `limit_reached`, on
    /// Foxhound's executable.
    pub fn run_start_crawl() -> Self {
        Self::read(RUN_START_CRAWL_CASES).expect("invariant: the embedded case list is valid")
    }

    /// `conformance/two-copies/cases.toml`: ADR 0003's fencing, a Run's
    /// copy that another resumed at a later run epoch seeing its
    /// Checkpoints and Exports refused by a destination that compares and
    /// sets, on Foxhound's executable.
    pub fn two_copies() -> Self {
        Self::read(TWO_COPIES_CASES).expect("invariant: the embedded case list is valid")
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
    /// The Daemon answered every case but did not take the harness's
    /// `daemon.stop`.
    #[error("the Daemon did not take the harness's stop: {reason}")]
    NotStopped {
        /// Why.
        reason: String,
    },
    /// The case log did not become a document.
    #[error(transparent)]
    Write(#[from] WriteError),
    /// The Goodfetch home of gate mode's run with blocking off could not be
    /// set up.
    #[error("setting up the home of the run with blocking off: {source}")]
    BlockingOffHome {
        /// Why.
        source: std::io::Error,
    },
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
/// as another Product than the suite's, when the Proving ground cannot serve
/// its task list or say which network fixtures it serves, and when a Daemon
/// the harness started does not take its stop, which gives up the suite's
/// document.
pub fn run_protocol_suite(
    suite: &ProtocolSuite,
    target: &EngineTarget,
    proving_ground: &ProvingGround,
) -> Result<ConformanceDocument, ProtocolSuiteError> {
    let task_list = proving_ground.task_list()?;
    let mut run = SuiteRun::start(suite.product, target, Site::of(&task_list, proving_ground)?)?;
    for (capability, declared) in declared_cases(suite) {
        run.take(capability, declared);
    }
    let ran = run.finish()?;
    let header = header(suite, target, &ran.hello, task_list, proving_ground);
    Ok(write_document(header, &suite.cases, &ran.log)?)
}

/// The folder, in the target's, of the Goodfetch home the second run of
/// gate mode starts the target on.
const BLOCKING_OFF_HOME: &str = "gate-blocking-off";

/// Runs `suite` as [`run_protocol_suite`] does, in gate mode (ADR 0047):
/// on the target as given, with Filter-list blocking on, and on a second
/// engine started on a Goodfetch home of the target's own, configured as
/// the target's is, with every Run's Filter lists off. The two runs are
/// interleaved: each case runs with blocking on, then at once with it off,
/// before the next, so both meet the outside world as it is at that
/// moment, and a vendor's outage falls on both runs of a case or on
/// neither far more often than on one alone. The document keeps each
/// case's entry of each run, marked with its blocking, and says it is a
/// gate run, so a case that fails only with blocking on fails, and the
/// document blocks.
///
/// # Errors
///
/// As [`run_protocol_suite`]'s, for either run, and when the second run's
/// home cannot be set up.
pub fn run_protocol_suite_in_gate_mode(
    suite: &ProtocolSuite,
    target: &EngineTarget,
    proving_ground: &ProvingGround,
) -> Result<ConformanceDocument, ProtocolSuiteError> {
    let task_list = proving_ground.task_list()?;
    // The second home is set up first, so it holds the target's
    // configuration as given, not as the first run's cases left it.
    let elsewhere = target
        .on_a_home_of_its_own(BLOCKING_OFF_HOME)
        .map_err(|source| ProtocolSuiteError::BlockingOffHome { source })?;
    let on_site = Site::of(&task_list, proving_ground)?.with_blocking(Blocking::On);
    let off_site = Site::of(&task_list, proving_ground)?.with_blocking(Blocking::Off);
    let mut on = SuiteRun::start(suite.product, target, on_site)?;
    let mut off = match SuiteRun::start(suite.product, &elsewhere, off_site) {
        Ok(off) => off,
        Err(refused) => {
            // The refusal is the failure reported; the first engine still
            // stops.
            drop(on.finish());
            return Err(refused);
        }
    };
    for (capability, declared) in declared_cases(suite) {
        on.take(capability, declared);
        off.take(capability, declared);
    }
    // Both engines stop before either's failure to stop is reported.
    let (on, off) = (on.finish(), off.finish());
    let (on, off) = (on?, off?);
    let mut log = on.log;
    log.extend(off.log);
    let header = ConformanceHeader {
        gate: true,
        ..header(suite, target, &on.hello, task_list, proving_ground)
    };
    Ok(write_document(header, &suite.cases, &log)?)
}

/// What one run of a suite's cases left: the target's `hello`, and the case
/// log.
struct Ran {
    hello: HelloResult,
    log: Vec<CaseLogEntry>,
}

/// The header of `suite`'s document, run on `target`, which answered
/// `hello`, against `proving_ground`, which served `task_list`.
fn header(
    suite: &ProtocolSuite,
    target: &EngineTarget,
    hello: &HelloResult,
    task_list: TaskList,
    proving_ground: &ProvingGround,
) -> ConformanceHeader {
    ConformanceHeader {
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
        gate: false,
    }
}

/// Each case `suite` declares, in its case list's order, with its
/// capability's name.
fn declared_cases(suite: &ProtocolSuite) -> impl Iterator<Item = (&str, &DeclaredCase)> {
    suite.cases.capabilities.iter().flat_map(|capability| {
        capability
            .cases
            .iter()
            .map(|declared| (capability.name.as_str(), declared))
    })
}

/// One run of a suite's cases on a target, over one Connection, against a
/// site: it starts, takes the cases one at a time, so gate mode can
/// interleave two runs, and finishes with their case log.
struct SuiteRun {
    /// The target as started, with the Proving ground's proxy password.
    target: EngineTarget,
    reached: Site,
    connection: Connection,
    hello: HelloResult,
    /// What `hello` exchanged, which the `hello` case's evidence opens with.
    opening: Vec<Value>,
    /// Why the Connection is gone, after which no case runs.
    gone: Option<Gone>,
    log: Vec<CaseLogEntry>,
}

impl SuiteRun {
    /// Starts `target` and opens its Connection with `hello`, announcing
    /// `product`, for cases run against `reached`.
    ///
    /// # Errors
    ///
    /// When the target cannot be reached, refuses or misreads `hello`, or
    /// runs as another Product.
    fn start(
        product: Product,
        target: &EngineTarget,
        reached: Site,
    ) -> Result<Self, ProtocolSuiteError> {
        // The harness gives the target the password of the Proving ground's
        // proxies, which a case's Session reads through the `env:` Secret
        // provider, as a program's own environment would hold it.
        let mut target = target.clone();
        if let Some(credentials) = reached.proxy_credentials() {
            target.environment.insert(
                PROXY_PASSWORD_VARIABLE.to_owned(),
                credentials.password.clone(),
            );
        }
        let mut connection = Connection::new(target::Started::start(&target)?);
        let hello = match hello(&mut connection, product) {
            Ok(hello) => hello,
            Err(refused) => {
                if matches!(refused, ProtocolSuiteError::OtherProduct { .. }) {
                    // The refusal is the failure reported; closing the
                    // target stops a Daemon that does not take the stop
                    // either.
                    drop(stop(&mut connection, target.mode));
                }
                connection.close();
                return Err(refused);
            }
        };
        let opening = connection.take_exchanged();
        Ok(Self {
            target,
            reached,
            connection,
            hello,
            opening,
            gone: None,
            log: Vec::new(),
        })
    }

    /// Runs `declared`, a case of `capability`, and logs it with the site's
    /// Filter-list blocking; once the Connection is gone, it runs nothing.
    fn take(&mut self, capability: &str, declared: &DeclaredCase) {
        if self.gone.is_some() {
            return;
        }
        let case = Case::of(&declared.id).expect("invariant: a read suite names known cases");
        let mut checks = Checks::default();
        let ran = case.run(
            &mut self.connection,
            &self.target,
            &self.hello,
            &self.reached,
            &mut checks,
        );
        checks.checks.extend(self.reached.take_unread());
        checks.checks.extend(self.connection.take_unanswered());
        if let Err(why) = ran {
            checks.checks.push(CaseCheck {
                name: format!("the target answers the case's calls ({why})"),
                passed: false,
            });
            self.gone = Some(why);
        }
        let mut messages = self.connection.take_exchanged();
        if case == Case::Hello {
            messages.splice(0..0, self.opening.iter().cloned());
        }
        self.log.push(CaseLogEntry {
            case: declared.id.clone(),
            capability: capability.to_owned(),
            mode: Some(self.target.mode.as_str().to_owned()),
            blocking: self.reached.blocking(),
            unreachable: Vec::new(),
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

    /// Stops a Daemon the harness started and closes the target.
    ///
    /// # Errors
    ///
    /// [`ProtocolSuiteError::NotStopped`] when a Daemon that answered every
    /// case does not take the stop.
    fn finish(mut self) -> Result<Ran, ProtocolSuiteError> {
        // A Connection that only fell silent may still take the stop, which
        // its failed case already reports when it does not; a Daemon that
        // answered every case must take it.
        let stopped = match self.gone {
            None => stop(&mut self.connection, self.target.mode),
            Some(Gone::Silent(_)) => {
                drop(stop(&mut self.connection, self.target.mode));
                Ok(())
            }
            Some(_) => Ok(()),
        };
        self.connection.close();
        stopped.map_err(|reason| ProtocolSuiteError::NotStopped { reason })?;
        Ok(Ran {
            hello: self.hello,
            log: self.log,
        })
    }
}

/// Stops a Daemon the harness started, over its open Connection, so it
/// removes its runtime file and socket.
///
/// # Errors
///
/// Why the Daemon did not take the stop: the Connection failed, or the
/// Daemon answered it with a failure. Closing the target stops a Daemon the
/// harness started either way.
fn stop(connection: &mut Connection, mode: EngineMode) -> Result<(), String> {
    if mode != EngineMode::Daemon {
        return Ok(());
    }
    let answer = connection
        .request(STOP.name, &json!({}))
        .and_then(|id| connection.answer(id))
        .map_err(|why| why.to_string())?;
    match answer.message.get("error") {
        Some(error) => Err(format!("it answered `{}` with {error}", STOP.name)),
        None => Ok(()),
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
        needs: Some(product),
        ..connection.hello_params(surface)
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
