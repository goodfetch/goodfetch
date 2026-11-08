//! The Cloud-readiness conformance suite's *gateway* target kind (spec #586,
//! ADRs 0068 and 0096): a gateway reached by address, which is the Stub
//! gateway in this repository and Goodfetch Cloud's gateway from its private
//! one, so both prove the same cases. Each case opens its own Connections as
//! the Clients it names, each opened with a `hello` that presents the
//! gateway's token: a Collie or a Foxhound Client, whose runs the routing
//! rule gives the Product its `hello` needs, a model's Client through the
//! MCP server, or the Console, whose runs it gives the Product their target
//! names. A case tells which engine a run landed on from what that engine
//! answers (`not_in_product` naming Collie and no install, worded for the
//! gateway, comes only from Collie's engine behind the gateway, an answer
//! to a Foxhound-only call only from Foxhound's), so it asserts only what a
//! Client observes. Every message of every Connection lands in the case's
//! evidence, a token redacted, and the writer turns the log into one
//! `conformance` document for the target.

use std::fmt;

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use goodfetch_protocol::connection::{
    HELLO, HelloParams, HelloResult, NOT_IN_PRODUCT, PROTOCOL_VERSION_UNSUPPORTED, Surface,
};
use goodfetch_protocol::page::session::RUN_NEW_SESSION;
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, Suite, Target, TargetIdentity,
};
use goodfetch_protocol::run::{RUN_CONFLICT, RUN_START, RUN_USAGE};
use goodfetch_protocol::settings::{SETTING_LOCKED, SETTING_NOT_APPLICABLE};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use url::{Host, Url};

use super::connection::{Connection, Gone, code_of};
use super::foxhound::{CRAWL_MAX_REQUESTS, CRAWL_START, RUN_SEEDS};
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
#[derive(Clone, Debug)]
pub struct GatewayTarget {
    /// Its name in the result document, such as `goodfetch-stub-gateway`.
    pub name: String,
    /// Where it is.
    pub address: GatewayAddress,
    /// The token a program presents in its `hello`, which only that
    /// `hello` carries and its `Debug` never shows.
    pub token: SecretString,
    /// The certificates (DER) its TLS may chain to besides Mozilla's roots,
    /// such as a staging gateway's private authority; none for most.
    pub trusted_certificates: Vec<Vec<u8>>,
}

/// A gateway's address: its `http` or `https` origin, such as
/// `https://gateway.example` or `http://127.0.0.1:4100`, under whose path
/// its WebSocket sits at `protocol`, `ws` or `wss` as the scheme says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayAddress(Url);

/// Text that is no gateway's address.
#[derive(Debug, thiserror::Error)]
#[error("`{text}` is no gateway's address: {reason}")]
pub struct GatewayAddressError {
    text: String,
    reason: String,
}

impl GatewayAddress {
    /// Reads `text`, an `http` or `https` URL with a host, and no
    /// credentials, query or fragment.
    ///
    /// # Errors
    ///
    /// When `text` is no such URL.
    pub fn parse(text: &str) -> Result<Self, GatewayAddressError> {
        let refused = |reason: &str| GatewayAddressError {
            text: text.to_owned(),
            reason: reason.to_owned(),
        };
        let mut url = Url::parse(text).map_err(|error| refused(&error.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(refused("its scheme is neither `http` nor `https`"));
        }
        if url.host().is_none() {
            return Err(refused("it names no host"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(refused("it carries credentials: the token goes in `hello`"));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(refused("it carries a query or a fragment"));
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self(url))
    }

    /// Whether the gateway speaks TLS.
    pub(crate) fn is_tls(&self) -> bool {
        self.0.scheme() == "https"
    }

    /// Its host, as a name or an address to connect to.
    pub(crate) fn host(&self) -> String {
        match self
            .0
            .host()
            .expect("invariant: a read gateway address names a host")
        {
            Host::Domain(name) => name.to_owned(),
            Host::Ipv4(address) => address.to_string(),
            Host::Ipv6(address) => address.to_string(),
        }
    }

    /// Its port, the scheme's own when it names none.
    pub(crate) fn port(&self) -> u16 {
        self.0
            .port_or_known_default()
            .expect("invariant: `http` and `https` have known ports")
    }

    /// The URL of its WebSocket.
    pub(crate) fn websocket(&self) -> Url {
        let mut websocket = self
            .0
            .join("protocol")
            .expect("invariant: `protocol` joins any gateway address");
        websocket
            .set_scheme(if self.is_tls() { "wss" } else { "ws" })
            .expect("invariant: `ws` and `wss` replace `http` and `https`");
        websocket
    }
}

impl fmt::Display for GatewayAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
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
    let mut connection = Connection::new(Started::gateway(target)?);
    let hello = hello_as(&mut connection, product, surface);
    Ok((connection, hello))
}

/// A Collie Client's `hello` that speaks the protocol minor after this
/// one's, which the version rule refuses during previews and accepts no
/// newer minor of after (ADRs 0030 and 0066).
fn hello_of_a_newer_minor(connection: &Connection) -> HelloParams {
    HelloParams {
        protocol_version: ProtocolVersion {
            minor: PROTOCOL_VERSION.minor + 1,
            ..PROTOCOL_VERSION
        },
        needs: Some(Product::Collie),
        ..connection.hello_params(Surface::Rust)
    }
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
    /// The MCP server, a Client acting for a model, that needs Collie.
    Model,
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
            As::Model => (Product::Collie, Surface::Mcp),
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

    /// The answer to the `hello` `params` makes of a new Connection, on it.
    fn hello_with(
        &mut self,
        params: impl FnOnce(&Connection) -> HelloParams,
    ) -> Result<Value, Gone> {
        let mut connection = Connection::new(
            Started::gateway(self.target).map_err(|error| Gone::Failed(error.to_string()))?,
        );
        let params =
            serde_json::to_value(params(&connection)).expect("invariant: `hello` serializes");
        let answer = connection
            .request(HELLO, &params)
            .and_then(|id| connection.first_answer(id));
        self.messages.extend(connection.take_exchanged());
        connection.close();
        Ok(answer?.message)
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
        let answer = self.call(foxhound, RUN_SEEDS, &json!({ "run_id": run_id }))?;
        Ok(if answer.get("result").is_some() {
            Some(Product::Foxhound)
        } else if collies_refusal_behind_the_gateway(&answer, RUN_SEEDS) {
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
/// and Foxhound as what the call needs, and no install command, which would
/// not help behind a gateway (ADR 0096) and which every engine not behind
/// one names in `install`, whatever its channel.
fn collies_refusal_behind_the_gateway(answer: &Value, requested: &str) -> bool {
    let data = &answer["error"]["data"];
    data["code"] == NOT_IN_PRODUCT.name
        && data["product"] == Product::Collie.as_str()
        && data["needs"] == Product::Foxhound.as_str()
        && data["requested"] == requested
        && data.get("install").is_none()
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
    OtherProtocolVersion,
    ModelsClient,
    CollieClientsRun,
    FoxhoundClientsRun,
    ConsoleRunOfCollie,
    ConsoleCrawlStart,
    ConsoleCrawlSetting,
    KnownRunId,
    RawCrawlStart,
    CrawlSetting,
    SessionCrawlSetting,
    ReadsAFoxhoundRun,
    CallACollieRunLacks,
}

impl GatewayCase {
    fn of(id: &str) -> Option<Self> {
        Some(match id {
            "a-collie-client-and-a-foxhound-client-are-each-answered-as-their-product" => {
                Self::ClientsPerProduct
            }
            "a-client-of-another-protocol-version-is-refused-at-hello" => {
                Self::OtherProtocolVersion
            }
            "a-models-client-is-held-to-a-models-settings-behind-the-gateway" => Self::ModelsClient,
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
            "a-collie-programs-crawl-setting-in-a-sessions-settings-answers-not-in-product-naming-the-runs-product" => {
                Self::SessionCrawlSetting
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
            Self::OtherProtocolVersion => {
                let answer = run.hello_with(hello_of_a_newer_minor)?;
                run.check(
                    "a Client's `hello` of another protocol minor answers `protocol_version_unsupported`, as an engine's does",
                    code_of(&answer) == PROTOCOL_VERSION_UNSUPPORTED.name,
                );
                Ok(())
            }
            Self::ModelsClient => models_client(run),
            Self::CollieClientsRun => {
                let collie = run.client(As::Program(Product::Collie))?;
                let run_id = run.start(collie, RUN_START.name, &json!({}))?;
                run.landed_on(run_id.as_deref(), Product::Collie)
            }
            Self::FoxhoundClientsRun => {
                let foxhound = run.client(As::Program(Product::Foxhound))?;
                let run_id = run.start(foxhound, RUN_START.name, &json!({}))?;
                run.landed_on(run_id.as_deref(), Product::Foxhound)
            }
            Self::ConsoleRunOfCollie => {
                let console = run.client(As::Console)?;
                let run_id = run.start(console, RUN_START.name, &json!({}))?;
                run.landed_on(run_id.as_deref(), Product::Collie)
            }
            Self::ConsoleCrawlStart => {
                let console = run.client(As::Console)?;
                let run_id = run.start(console, CRAWL_START, &crawl_of_nothing())?;
                run.landed_on(run_id.as_deref(), Product::Foxhound)
            }
            Self::ConsoleCrawlSetting => {
                // Foxhound's engine carries the key and refuses it on a run
                // that is no crawl; Collie's would refuse it as Foxhound's.
                let console = run.client(As::Console)?;
                let answer = run.call(
                    console,
                    RUN_START.name,
                    &json!({"settings": {"crawl": {"max_requests": 10}}}),
                )?;
                run.check(
                    "Foxhound's engine answers the start, refusing the crawl key on a run that is no crawl with `setting_not_applicable`",
                    code_of(&answer) == SETTING_NOT_APPLICABLE.name,
                );
                Ok(())
            }
            Self::KnownRunId => known_run_id(run),
            Self::RawCrawlStart => {
                let collie = run.client(As::Program(Product::Collie))?;
                let answer = run.call(collie, CRAWL_START, &crawl_of_nothing())?;
                run.check(
                    "Collie's engine refuses `crawl.start` with `not_in_product` naming the run's Product, Collie, and no install",
                    collies_refusal_behind_the_gateway(&answer, CRAWL_START),
                );
                Ok(())
            }
            Self::CrawlSetting => {
                let collie = run.client(As::Program(Product::Collie))?;
                let answer = run.call(
                    collie,
                    RUN_START.name,
                    &json!({"settings": {"crawl": {"max_requests": 10}}}),
                )?;
                run.check(
                    "Collie's engine refuses the crawl key with `not_in_product` naming the run's Product, Collie, and no install",
                    collies_refusal_behind_the_gateway(&answer, CRAWL_MAX_REQUESTS),
                );
                Ok(())
            }
            Self::SessionCrawlSetting => {
                let collie = run.client(As::Program(Product::Collie))?;
                let run_id = run.start(collie, RUN_START.name, &json!({}))?;
                let answer = run.call(
                    collie,
                    RUN_NEW_SESSION.name,
                    &json!({"run_id": run_id, "settings": {"crawl": {"max_requests": 10}}}),
                )?;
                run.check(
                    "Collie's engine refuses the crawl key in a Session's own settings with `not_in_product` naming the run's Product, Collie, and no install",
                    collies_refusal_behind_the_gateway(&answer, CRAWL_MAX_REQUESTS),
                );
                Ok(())
            }
            Self::ReadsAFoxhoundRun => reads_a_foxhound_run(run),
            Self::CallACollieRunLacks => {
                let collie = run.client(As::Program(Product::Collie))?;
                let run_id = run.start(collie, RUN_START.name, &json!({}))?;
                let foxhound = run.client(As::Program(Product::Foxhound))?;
                let answer = run.call(foxhound, RUN_SEEDS, &json!({ "run_id": run_id }))?;
                run.check(
                    "a Foxhound Client's `run.seeds` on a run of Collie's engine answers `not_in_product` naming the run's Product, Collie",
                    collies_refusal_behind_the_gateway(&answer, RUN_SEEDS),
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
        Err(refused) => refused["data"]["code"] == NOT_IN_PRODUCT.name,
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
/// which a new Run ID would put on Collie's. Foxhound's engine answers that
/// start, whose inputs differ from the run's, with `run_conflict`; a new
/// engine would start a run of its own.
fn known_run_id(run: &mut CaseRun<'_>) -> Result<(), Gone> {
    let console = run.client(As::Console)?;
    let run_id = run.start(console, CRAWL_START, &crawl_of_nothing())?;
    let Some(run_id) = run_id else {
        return Ok(());
    };
    let again = run.call(console, RUN_START.name, &json!({ "run_id": run_id }))?;
    run.check(
        "the start of the known Run ID reaches its run, which answers `run_conflict` for the start's other inputs",
        code_of(&again) == RUN_CONFLICT.name
            && again["error"]["data"]["run_id"] == run_id.as_str(),
    );
    run.landed_on(Some(&run_id), Product::Foxhound)
}

/// A Collie Client reaches a run a Foxhound Client started, and reads its
/// Collie-shaped state; a call the run's engine carries is answered to it
/// by that engine, which never narrows its catalog for a Client (ADRs 0095
/// and 0096).
fn reads_a_foxhound_run(run: &mut CaseRun<'_>) -> Result<(), Gone> {
    let foxhound = run.client(As::Program(Product::Foxhound))?;
    let run_id = run.start(foxhound, CRAWL_START, &crawl_of_nothing())?;
    let Some(run_id) = run_id else {
        return Ok(());
    };
    let collie = run.client(As::Program(Product::Collie))?;
    let usage = run.call(collie, RUN_USAGE.name, &json!({ "run_id": run_id }))?;
    run.check(
        "a Collie Client reads the Foxhound run's Usage",
        usage.get("result").is_some(),
    );
    let seeds = run.call(collie, RUN_SEEDS, &json!({ "run_id": run_id }))?;
    run.check(
        "its `run.seeds`, which the run's engine carries, is answered",
        seeds.get("result").is_some(),
    );
    Ok(())
}

/// A model's Client, the MCP server's, meets behind the gateway the rules
/// it meets on a local engine (ADR 0092): it sets no proxy, whose setter
/// receives the proxy's password, on a run or on a Session, each refused
/// with `setting_locked`.
fn models_client(run: &mut CaseRun<'_>) -> Result<(), Gone> {
    let proxy = json!({"proxy": {"server": "http://proxy.example:3128"}});
    let model = run.client(As::Model)?;
    let start = run.call(model, RUN_START.name, &json!({ "settings": proxy }))?;
    run.check(
        "a model's `run.start` setting `proxy.server` answers `setting_locked`",
        code_of(&start) == SETTING_LOCKED.name,
    );
    let run_id = run.start(model, RUN_START.name, &json!({}))?;
    let session = run.call(
        model,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": proxy }),
    )?;
    run.check(
        "a model's `run.new_session` setting `proxy.server` answers `setting_locked`",
        code_of(&session) == SETTING_LOCKED.name,
    );
    Ok(())
}

#[cfg(test)]
mod tests;
