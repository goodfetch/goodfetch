//! The Cloud-readiness conformance suite's *gateway* target kind (spec #586,
//! ADRs 0068 and 0096): a gateway reached by address, which is the Stub
//! gateway in this repository and Goodfetch Cloud's gateway from its private
//! one, so both prove the same cases. Each case opens its own Connections as
//! the Clients it names, each signed in with the gateway's token and opened
//! with `hello`: a Collie or a Foxhound Client, whose runs the routing rule
//! gives the Product its `hello` needs, or the Console, whose runs it gives
//! the Product their target names. A case tells which engine a run landed on
//! from what that engine answers (`not_in_product` worded for the gateway
//! and naming Collie comes only from Collie's engine behind the gateway, an
//! answer to a Foxhound-only call only from Foxhound's), so it asserts only
//! what a Client observes. Every message of every Connection lands in the
//! case's evidence, and the writer turns the log into one `conformance`
//! document for the target.

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HelloResult, Surface};
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, Suite, Target, TargetIdentity,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::connection::{Connection, Gone};
use super::target::{StartError, Started};
use super::{ProtocolSuiteError, SuiteError, hello_as};
use crate::case_list::{CaseList, DeclaredCase};
use crate::host::{RUNTIME, platform};
use crate::proving_ground::{ProvingGround, ProvingGroundError};
use crate::writer::{WriteError, write_document};

/// The Cloud-readiness suite's case list, embedded so the suite runs
/// wherever the harness does, Goodfetch Cloud's repository included.
const CASES: &str = include_str!("../../../../conformance/cloud-readiness/cases.toml");

/// What the case entries name as the mode a gateway case ran in.
const MODE: &str = "gateway";

/// The kind of target a capability's cases run against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TargetKind {
    /// A gateway, reached by address.
    Gateway,
}

/// The suite's case list as its file writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteCases {
    suite: Suite,
    #[serde(rename = "capability")]
    capabilities: Vec<ReadCapability>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCapability {
    name: String,
    target: TargetKind,
    #[serde(rename = "case")]
    cases: Vec<DeclaredCase>,
}

/// The gateway target kind's part of the Cloud-readiness suite: its
/// capabilities whose `target` is `gateway`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewaySuite {
    cases: CaseList,
}

impl GatewaySuite {
    /// `conformance/cloud-readiness/cases.toml`'s gateway cases.
    pub fn embedded() -> Self {
        Self::read(CASES).expect("invariant: the embedded case list is valid")
    }

    /// Reads the gateway cases of a Cloud-readiness case list.
    ///
    /// # Errors
    ///
    /// When the text is no case list, or a gateway case ID names no case the
    /// harness runs.
    pub fn read(text: &str) -> Result<Self, SuiteError> {
        let read: SuiteCases = toml::from_str(text).map_err(SuiteError::Malformed)?;
        let capabilities: Vec<_> = read
            .capabilities
            .into_iter()
            .filter(|capability| capability.target == TargetKind::Gateway)
            .map(|capability| crate::case_list::DeclaredCapability {
                name: capability.name,
                cases: capability.cases,
            })
            .collect();
        if let Some(case) = capabilities
            .iter()
            .flat_map(|capability| &capability.cases)
            .find(|case| GatewayCase::of(&case.id).is_none())
        {
            return Err(SuiteError::UnknownCase {
                case: case.id.clone(),
            });
        }
        Ok(Self {
            cases: CaseList {
                suite: read.suite,
                capabilities,
            },
        })
    }

    /// Its declared case list.
    pub fn case_list(&self) -> &CaseList {
        &self.cases
    }
}

/// A gateway, reached by address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayTarget {
    /// Its name in the result document, such as `goodfetch-stub-gateway`.
    pub name: String,
    /// Its origin, such as `http://127.0.0.1:4100`.
    pub address: String,
    /// The token a Client signs in with.
    pub token: String,
}

/// Why the gateway suite could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GatewaySuiteError {
    /// The gateway took no Connection, or refused a Collie Client's `hello`,
    /// which every gateway accepts.
    #[error(transparent)]
    Hello(#[from] ProtocolSuiteError),
    /// The Proving ground gave no usable answer.
    #[error(transparent)]
    ProvingGround(#[from] ProvingGroundError),
    /// The case log did not become a document.
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// Runs `suite` against the gateway `target`, beside the running
/// `proving_ground`, and returns its result document.
///
/// # Errors
///
/// When the gateway takes no Connection or refuses a Collie Client, and
/// when the Proving ground cannot serve its task list.
pub fn run_gateway_suite(
    suite: &GatewaySuite,
    target: &GatewayTarget,
    proving_ground: &ProvingGround,
) -> Result<ConformanceDocument, GatewaySuiteError> {
    let task_list = proving_ground.task_list()?;
    // The gateway's identity, from a Collie Client's `hello`, which every
    // gateway accepts.
    let (connection, hello) =
        open(target, Product::Collie, Surface::Rust).map_err(ProtocolSuiteError::from)?;
    connection.close();
    let hello = hello?;
    let mut log = Vec::new();
    for capability in &suite.cases.capabilities {
        for declared in &capability.cases {
            let case =
                GatewayCase::of(&declared.id).expect("invariant: a read suite names known cases");
            let mut run = CaseRun::new(target);
            if let Err(why) = case.run(&mut run) {
                run.check(
                    format!("the gateway answers the case's calls ({why})"),
                    false,
                );
            }
            let (checks, messages) = run.finish();
            log.push(CaseLogEntry {
                case: declared.id.clone(),
                capability: capability.name.clone(),
                mode: Some(MODE.to_owned()),
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
    let header = ConformanceHeader {
        suite: suite.cases.suite.clone(),
        target: Target {
            name: target.name.clone(),
            identity: TargetIdentity::Version(hello.product_version.to_string()),
            product: None,
        },
        platform: platform(),
        runtime: RUNTIME.to_owned(),
        proving_ground: task_list.proving_ground,
        online: proving_ground.is_online(),
    };
    Ok(write_document(header, &suite.cases, &log)?)
}

/// Opens a Connection to the gateway as a Client of `surface` that needs
/// `product`, and its `hello`'s answer.
fn open(
    target: &GatewayTarget,
    product: Product,
    surface: Surface,
) -> Result<(Connection, Result<HelloResult, ProtocolSuiteError>), StartError> {
    let mut connection = Connection::new(Started::gateway(&target.address, &target.token)?);
    let hello = hello_as(&mut connection, product, surface);
    Ok((connection, hello))
}

/// One case's run: its Connections, checks and messages.
struct CaseRun<'a> {
    target: &'a GatewayTarget,
    connections: Vec<Connection>,
    checks: Vec<CaseCheck>,
    messages: Vec<Value>,
}

/// One of the case's Connections, by its place in [`CaseRun`].
#[derive(Clone, Copy)]
struct Client(usize);

/// Who a case's Connection is.
#[derive(Clone, Copy)]
enum As {
    /// An SDK's Client that needs the Product.
    Program(Product),
    /// The Console, served Foxhound's build.
    Console,
}

impl<'a> CaseRun<'a> {
    fn new(target: &'a GatewayTarget) -> Self {
        Self {
            target,
            connections: Vec::new(),
            checks: Vec::new(),
            messages: Vec::new(),
        }
    }

    fn check(&mut self, name: impl Into<String>, passed: bool) {
        self.checks.push(CaseCheck {
            name: name.into(),
            passed,
        });
    }

    /// A Connection as `who`, its `hello` answered; `Err` with the refusal
    /// when the gateway refused it.
    fn connect(&mut self, who: As) -> Result<Result<Client, Value>, Gone> {
        let (product, surface) = match who {
            As::Program(product) => (product, Surface::Rust),
            As::Console => (Product::Foxhound, Surface::Console),
        };
        let (mut connection, hello) =
            open(self.target, product, surface).map_err(|error| Gone::Failed(error.to_string()))?;
        self.messages.extend(connection.take_exchanged());
        let refused = match hello {
            Ok(_) => {
                self.connections.push(connection);
                return Ok(Ok(Client(self.connections.len() - 1)));
            }
            Err(ProtocolSuiteError::HelloRefused { error }) => error,
            Err(ProtocolSuiteError::OtherProduct { engine, .. }) => {
                json!({ "answered_as": engine.name() })
            }
            Err(other) => {
                connection.close();
                return Err(Gone::Failed(other.to_string()));
            }
        };
        connection.close();
        Ok(Err(refused))
    }

    /// A Connection as `who`, which the gateway must accept.
    fn client(&mut self, who: As) -> Result<Client, Gone> {
        self.connect(who)?.map_err(|refused| {
            Gone::Failed(format!(
                "the gateway refused the Client's `hello`: {refused}"
            ))
        })
    }

    /// The whole answer to `method` with `params` on `client`.
    fn call(&mut self, client: Client, method: &str, params: &Value) -> Result<Value, Gone> {
        let connection = &mut self.connections[client.0];
        let id = connection.request(method, params)?;
        let answer = connection.first_answer(id);
        self.messages.extend(connection.take_exchanged());
        Ok(answer?.message)
    }

    /// The Run ID of a run `client` starts with `method` and `params`, and
    /// a check that it started.
    fn start(
        &mut self,
        client: Client,
        method: &str,
        params: &Value,
    ) -> Result<Option<String>, Gone> {
        let answer = self.call(client, method, params)?;
        let run_id = answer["result"]["run_id"].as_str().map(ToOwned::to_owned);
        self.check(format!("`{method}` starts a run"), run_id.is_some());
        Ok(run_id)
    }

    /// Which engine holds `run_id`, as a Foxhound Client finds it: Collie's
    /// answers `run.seeds` with `not_in_product` worded for the gateway,
    /// Foxhound's answers it.
    fn engine_of(&mut self, run_id: &str) -> Result<Option<Product>, Gone> {
        let foxhound = self.client(As::Program(Product::Foxhound))?;
        let answer = self.call(foxhound, "run.seeds", &json!({ "run_id": run_id }))?;
        Ok(if answer.get("result").is_some() {
            Some(Product::Foxhound)
        } else if collies_refusal_behind_the_gateway(&answer, "run.seeds") {
            Some(Product::Collie)
        } else {
            None
        })
    }

    /// Checks that the run `run_id` landed on `product`'s engine.
    fn landed_on(&mut self, run_id: Option<&str>, product: Product) -> Result<(), Gone> {
        let engine = match run_id {
            Some(run_id) => self.engine_of(run_id)?,
            None => None,
        };
        self.check(
            format!("the run landed on {}'s engine", product.name()),
            engine == Some(product),
        );
        Ok(())
    }

    fn finish(mut self) -> (Vec<CaseCheck>, Vec<Value>) {
        for mut connection in self.connections.drain(..) {
            self.messages.extend(connection.take_exchanged());
            connection.close();
        }
        (self.checks, self.messages)
    }
}

/// Whether `answer` is `not_in_product` for `requested` from Collie's
/// engine behind the gateway: its details name Collie as the run's Product
/// and Foxhound as what the call needs, and its `fix` names no install,
/// which would not help behind a gateway (ADR 0096).
fn collies_refusal_behind_the_gateway(answer: &Value, requested: &str) -> bool {
    let data = &answer["error"]["data"];
    data["code"] == "not_in_product"
        && data["product"] == "collie"
        && data["needs"] == "foxhound"
        && data["requested"] == requested
        && data["fix"]
            .as_str()
            .is_some_and(|fix| !fix.contains("install"))
}

/// A crawl's start whose one seed the Frontier does not add, so the crawl
/// fetches nothing and runs no handler.
fn crawl_of_nothing() -> Value {
    json!({"seeds": [{"url": "ftp://example.invalid/", "label": "start"}]})
}

/// A gateway case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GatewayCase {
    ClientsPerProduct,
    CollieClientsRun,
    FoxhoundClientsRun,
    ConsoleRunOfCollie,
    ConsoleCrawlStart,
    ConsoleCrawlSetting,
    KnownRunId,
    RawCrawlStart,
    CrawlSetting,
    ReadsAFoxhoundRun,
    CallACollieRunLacks,
}

impl GatewayCase {
    fn of(id: &str) -> Option<Self> {
        Some(match id {
            "a-collie-client-and-a-foxhound-client-are-each-answered-as-their-product" => {
                Self::ClientsPerProduct
            }
            "a-collie-clients-run-lands-on-collies-engine" => Self::CollieClientsRun,
            "a-foxhound-clients-run-lands-on-foxhounds-engine" => Self::FoxhoundClientsRun,
            "a-console-run-naming-nothing-only-foxhound-carries-lands-on-collies-engine" => {
                Self::ConsoleRunOfCollie
            }
            "a-console-crawl-start-lands-on-foxhounds-engine" => Self::ConsoleCrawlStart,
            "a-console-run-setting-a-crawl-key-lands-on-foxhounds-engine" => {
                Self::ConsoleCrawlSetting
            }
            "a-start-naming-a-known-run-id-goes-to-that-runs-engine" => Self::KnownRunId,
            "a-collie-programs-raw-crawl-start-answers-not-in-product-naming-the-runs-product" => {
                Self::RawCrawlStart
            }
            "a-collie-programs-crawl-setting-answers-not-in-product-naming-the-runs-product" => {
                Self::CrawlSetting
            }
            "a-collie-client-reads-a-foxhound-run-it-did-not-start" => Self::ReadsAFoxhoundRun,
            "a-foxhound-clients-call-a-collie-runs-engine-lacks-answers-not-in-product" => {
                Self::CallACollieRunLacks
            }
            _ => return None,
        })
    }

    fn run(self, run: &mut CaseRun<'_>) -> Result<(), Gone> {
        match self {
            Self::ClientsPerProduct => clients_per_product(run),
            Self::CollieClientsRun => {
                let collie = run.client(As::Program(Product::Collie))?;
                let run_id = run.start(collie, "run.start", &json!({}))?;
                run.landed_on(run_id.as_deref(), Product::Collie)
            }
            Self::FoxhoundClientsRun => {
                let foxhound = run.client(As::Program(Product::Foxhound))?;
                let run_id = run.start(foxhound, "run.start", &json!({}))?;
                run.landed_on(run_id.as_deref(), Product::Foxhound)
            }
            Self::ConsoleRunOfCollie => {
                let console = run.client(As::Console)?;
                let run_id = run.start(console, "run.start", &json!({}))?;
                run.landed_on(run_id.as_deref(), Product::Collie)
            }
            Self::ConsoleCrawlStart => {
                let console = run.client(As::Console)?;
                let run_id = run.start(console, "crawl.start", &crawl_of_nothing())?;
                run.landed_on(run_id.as_deref(), Product::Foxhound)
            }
            Self::ConsoleCrawlSetting => {
                // Foxhound's engine carries the key and refuses it on a run
                // that is no crawl; Collie's would refuse it as Foxhound's.
                let console = run.client(As::Console)?;
                let answer = run.call(
                    console,
                    "run.start",
                    &json!({"settings": {"crawl": {"max_requests": 10}}}),
                )?;
                run.check(
                    "Foxhound's engine answers the start, refusing the crawl key on a run that is no crawl with `setting_not_applicable`",
                    answer["error"]["data"]["code"] == "setting_not_applicable",
                );
                Ok(())
            }
            Self::KnownRunId => known_run_id(run),
            Self::RawCrawlStart => {
                let collie = run.client(As::Program(Product::Collie))?;
                let answer = run.call(collie, "crawl.start", &crawl_of_nothing())?;
                run.check(
                    "Collie's engine refuses `crawl.start` with `not_in_product` naming the run's Product, Collie, and no install",
                    collies_refusal_behind_the_gateway(&answer, "crawl.start"),
                );
                Ok(())
            }
            Self::CrawlSetting => {
                let collie = run.client(As::Program(Product::Collie))?;
                let answer = run.call(
                    collie,
                    "run.start",
                    &json!({"settings": {"crawl": {"max_requests": 10}}}),
                )?;
                run.check(
                    "Collie's engine refuses the crawl key with `not_in_product` naming the run's Product, Collie, and no install",
                    collies_refusal_behind_the_gateway(&answer, "crawl.max_requests"),
                );
                Ok(())
            }
            Self::ReadsAFoxhoundRun => reads_a_foxhound_run(run),
            Self::CallACollieRunLacks => {
                let collie = run.client(As::Program(Product::Collie))?;
                let run_id = run.start(collie, "run.start", &json!({}))?;
                let foxhound = run.client(As::Program(Product::Foxhound))?;
                let answer = run.call(foxhound, "run.seeds", &json!({ "run_id": run_id }))?;
                run.check(
                    "a Foxhound Client's `run.seeds` on a run of Collie's engine answers `not_in_product` naming the run's Product, Collie",
                    collies_refusal_behind_the_gateway(&answer, "run.seeds"),
                );
                Ok(())
            }
        }
    }
}

/// A Collie Client is accepted as Collie; a Foxhound Client as Foxhound, or,
/// by a gateway that cannot run Foxhound's engine, refused at `hello` with
/// `not_in_product`.
fn clients_per_product(run: &mut CaseRun<'_>) -> Result<(), Gone> {
    let collie = run.connect(As::Program(Product::Collie))?;
    run.check(
        "a Collie Client's `hello` is answered as Collie",
        collie.is_ok(),
    );
    let foxhound = run.connect(As::Program(Product::Foxhound))?;
    let passed = match &foxhound {
        Ok(_) => true,
        Err(refused) => refused["data"]["code"] == "not_in_product",
    };
    run.check(
        "a Foxhound Client's `hello` is answered as Foxhound, or refused with `not_in_product` by a gateway that cannot run Foxhound's engine",
        passed,
    );
    Ok(())
}

/// A start naming a Run ID the gateway's index holds goes to that run's
/// engine: a run the Console started on Foxhound's engine stays there when
/// the Console starts its Run ID again with nothing only Foxhound carries,
/// which a new Run ID would put on Collie's.
fn known_run_id(run: &mut CaseRun<'_>) -> Result<(), Gone> {
    let console = run.client(As::Console)?;
    let run_id = run.start(console, "crawl.start", &crawl_of_nothing())?;
    let Some(run_id) = run_id else {
        return Ok(());
    };
    run.call(console, "run.start", &json!({ "run_id": run_id }))?;
    run.landed_on(Some(&run_id), Product::Foxhound)
}

/// A Collie Client reaches a run a Foxhound Client started, and reads its
/// Collie-shaped state; a call only Foxhound carries is refused to it with
/// `not_in_product`, as a Collie engine refuses it.
fn reads_a_foxhound_run(run: &mut CaseRun<'_>) -> Result<(), Gone> {
    let foxhound = run.client(As::Program(Product::Foxhound))?;
    let run_id = run.start(foxhound, "crawl.start", &crawl_of_nothing())?;
    let Some(run_id) = run_id else {
        return Ok(());
    };
    let collie = run.client(As::Program(Product::Collie))?;
    let usage = run.call(collie, "run.usage", &json!({ "run_id": run_id }))?;
    run.check(
        "a Collie Client reads the Foxhound run's Usage",
        usage.get("result").is_some(),
    );
    let seeds = run.call(collie, "run.seeds", &json!({ "run_id": run_id }))?;
    let data = &seeds["error"]["data"];
    run.check(
        "its `run.seeds` answers `not_in_product`, needing Foxhound",
        data["code"] == "not_in_product" && data["needs"] == "foxhound",
    );
    Ok(())
}
