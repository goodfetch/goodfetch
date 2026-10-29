//! The composition (ADR 0087): each domain crate exports one
//! [`Contribution`] listing what it adds to the engine, and each Product's
//! executable crate passes the ones it links to the engine at start, a list
//! in its `main`. Nothing registers itself by being linked. Today a
//! contribution adds operation handlers and the tables it keeps in every
//! run file, and an executable crate adds its Console build; settings keys, Plugin kinds and Hook stages, Usage kinds and
//! Limit scopes, Notifications and gates in admission join it with the
//! tickets that build them.
//!
//! A handler answers a value or a [`CallError`], which only the RPC layer
//! turns into the wire's error object.

mod call_streams;
mod params;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use goodfetch_protocol::catalog::{ErrorCodeDefinition, Operation, Setting, SharedTypeValue};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::daemon::{DaemonHealth, DaemonState, DaemonStopping};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use tokio_util::sync::CancellationToken;

use crate::admission::Gates;
use crate::client::Client;
use crate::controls::Controls;
use crate::events::Events;
use crate::log::ConnectionLog;
use crate::needs_you::NeedsYouSource;
use crate::people::SignInDriver;
use crate::politeness::PolitenessTables;
use crate::product::Identity;
use crate::run_state::RunStateTables;
use crate::runs::{CleanUpVersions, ProgressPart, ReportPart, Run, Runs};
use crate::secrets::ProviderFactory;
use crate::settings::RunSettings;
use goodfetch_protocol::run::RunKind;
use goodfetch_protocol::secret_reference::Provider;

pub use call_streams::{CallInput, CallStream};
pub(crate) use call_streams::{CallNotifier, InputSender};

/// A failure a call answers with: an Error code of the catalog, with its
/// message and details, or a failure of the protocol itself.
#[derive(Clone)]
pub struct CallError(Box<Failure>);

#[derive(Clone)]
enum Failure {
    Code {
        definition: &'static ErrorCodeDefinition,
        message: String,
        details: Map<String, Value>,
    },
    Protocol {
        failure: ProtocolFailure,
        message: String,
    },
}

impl CallError {
    /// A failure with an Error code of the catalog and the code's details,
    /// which serialize to a JSON object.
    pub fn new(
        definition: &'static ErrorCodeDefinition,
        message: impl Into<String>,
        details: &impl Serialize,
    ) -> Self {
        let details = match serde_json::to_value(details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        };
        Self(Box::new(Failure::Code {
            definition,
            message: message.into(),
            details,
        }))
    }

    /// A failure of the protocol itself, such as params that are not the
    /// operation's.
    pub fn protocol(failure: ProtocolFailure, message: impl Into<String>) -> Self {
        Self(Box::new(Failure::Protocol {
            failure,
            message: message.into(),
        }))
    }

    /// Its Error code, or the protocol failure's name.
    pub fn code(&self) -> &'static str {
        match &*self.0 {
            Failure::Code { definition, .. } => definition.name,
            Failure::Protocol { failure, .. } => failure.name(),
        }
    }

    /// Its Error code's details; none for a protocol failure.
    pub fn details(&self) -> Option<&Map<String, Value>> {
        match &*self.0 {
            Failure::Code { details, .. } => Some(details),
            Failure::Protocol { .. } => None,
        }
    }

    /// What it says, for people.
    pub fn message(&self) -> &str {
        match &*self.0 {
            Failure::Code { message, .. } | Failure::Protocol { message, .. } => message,
        }
    }

    /// Whether a retry may succeed, as its Error code says; never for a
    /// protocol failure.
    pub fn retryable(&self) -> bool {
        match &*self.0 {
            Failure::Code { definition, .. } => definition.retryable,
            Failure::Protocol { .. } => false,
        }
    }

    /// Its numeric code on the wire: a predefined JSON-RPC code or its
    /// group's class.
    pub(crate) fn wire_code(&self) -> i32 {
        match &*self.0 {
            Failure::Code { definition, .. } => definition.group.json_rpc_class(),
            Failure::Protocol { failure, .. } => failure.code(),
        }
    }

    /// Its numeric class on the wire, whether a retry may succeed and its
    /// details, for the RPC layer.
    pub(crate) fn wire_parts(&self) -> (i32, bool, Map<String, Value>) {
        match &*self.0 {
            Failure::Code {
                definition,
                details,
                ..
            } => (
                definition.group.json_rpc_class(),
                definition.retryable,
                details.clone(),
            ),
            Failure::Protocol { failure, .. } => (failure.code(), false, Map::new()),
        }
    }
}

impl fmt::Debug for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (_, retryable, details) = self.wire_parts();
        formatter
            .debug_struct("CallError")
            .field("code", &self.code())
            .field("message", &self.message())
            .field("retryable", &retryable)
            .field("details", &details)
            .finish()
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for CallError {}

/// What a handler is given beside its params: its Connection's handles,
/// shared by every call of that Connection, its cancellation, and for a
/// streaming call its own stream of notifications. A call clones one handle
/// of its Connection, so its boxed answer stays small (ADR 0134's figures
/// per dispatched message).
#[derive(Clone)]
pub struct CallContext {
    connection: Arc<ConnectionContext>,
    id: RequestId,
    token: CancellationToken,
    stream: Option<Arc<CallStream>>,
    input: Option<Arc<CallInput>>,
}

/// What every call of one Connection is given: the engine's identity, the
/// Connection's Client and the surface its `hello` named, the engine's
/// Runs and event stream, and on a Daemon the Daemon itself. The protocol
/// server builds it once per Connection and sets the surface at `hello`.
pub(crate) struct ConnectionContext {
    pub(crate) identity: Arc<Identity>,
    pub(crate) client: Client,
    pub(crate) surface: OnceLock<Surface>,
    /// Whether its `hello` came from Goodfetch Cloud's gateway, which words
    /// `not_in_product` for it (ADR 0096); set with the surface.
    pub(crate) behind_gateway: OnceLock<bool>,
    /// The driver of each kind of Run that has one, which a resume starts
    /// again.
    pub(crate) drivers: Arc<BTreeMap<RunKind, Driver>>,
    pub(crate) runs: Runs,
    pub(crate) events: Events,
    pub(crate) daemon: Option<Arc<dyn DaemonSide>>,
    /// The log events the Connection receives, at the level it asks for.
    pub(crate) log: ConnectionLog,
}

/// A boxed future of a [`DaemonSide`]'s answer.
pub(crate) type Answering<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a Daemon gives each of its Connections (ADRs 0024 and 0098): the
/// token every `hello` carries, the count of its Clients, whether it hands
/// over on a refusal, and its answers to `daemon.*`.
pub(crate) trait DaemonSide: Send + Sync {
    /// The per-launch token.
    fn token(&self) -> &str;
    /// A Client of `surface` opened; it counts until the value drops.
    fn opened(&self, surface: Surface) -> Box<dyn Send + Sync>;
    /// A `hello` is being refused by Product or version: whether the Daemon
    /// hands over to its major's pick on it, which it has then begun.
    fn refusing(&self) -> Answering<'_, bool>;
    /// What `daemon.status` answers.
    fn status(&self) -> Answering<'_, DaemonState>;
    /// What `daemon.health` answers.
    fn health(&self) -> Answering<'_, DaemonHealth>;
    /// Stops the Daemon once `daemon.stop` has answered what it says.
    fn stop(&self) -> Answering<'_, DaemonStopping>;
    /// The sign-in driver of the Console it serves, if any.
    fn sign_ins(&self) -> Option<SignInDriver>;
}

impl CallContext {
    /// The call `id` on `connection`, cancelled through `token`, with
    /// `stream` when its handler streams, and `input` when it takes input.
    pub(crate) fn new(
        connection: Arc<ConnectionContext>,
        id: RequestId,
        token: CancellationToken,
        stream: Option<CallStream>,
        input: Option<CallInput>,
    ) -> Self {
        Self {
            connection,
            id,
            token,
            stream: stream.map(Arc::new),
            input: input.map(Arc::new),
        }
    }

    /// The call's id, which the handler requests it sends its Client may
    /// name, such as a route's `route.handle`.
    pub fn id(&self) -> &RequestId {
        &self.id
    }

    /// The call's stream of input from its Client, which only a handler
    /// built with [`Handler::taking_input`] reads.
    ///
    /// # Panics
    ///
    /// When the handler takes no input: its calls carry none.
    pub fn input(&self) -> &CallInput {
        self.input
            .as_deref()
            .expect("invariant: a handler taking input gets its calls' input")
    }

    /// The engine's event stream, on which a handler publishes its events.
    pub fn events(&self) -> &Events {
        &self.connection.events
    }

    /// The call's own stream of notifications, which only a handler built
    /// with [`Handler::streaming`] reads.
    ///
    /// # Panics
    ///
    /// When the handler does not stream: its calls carry no stream.
    pub fn stream(&self) -> &CallStream {
        self.stream
            .as_deref()
            .expect("invariant: a streaming handler's calls carry their stream")
    }

    /// The Client of the call's Connection, to which a Run started by the
    /// call sends its handler requests.
    pub fn client(&self) -> &Client {
        &self.connection.client
    }

    /// The surface the Connection's `hello` named, in whose language the
    /// engine renders the names and targets its errors show (ADRs 0079 and
    /// 0099).
    pub fn surface(&self) -> Surface {
        self.connection
            .surface
            .get()
            .copied()
            .expect("invariant: a call runs only on a Connection whose `hello` was answered")
    }

    /// Whether the Connection's `hello` came from Goodfetch Cloud's gateway
    /// (or the Stub gateway), on whose runs `not_in_product` names the run's
    /// Product rather than an install (ADR 0096).
    pub fn behind_gateway(&self) -> bool {
        self.connection
            .behind_gateway
            .get()
            .copied()
            .unwrap_or(false)
    }

    /// The driver of Runs of `kind`, when the composition has one.
    pub fn driver(&self, kind: RunKind) -> Option<Driver> {
        self.connection.drivers.get(&kind).cloned()
    }

    /// The engine's Runs.
    pub fn runs(&self) -> &Runs {
        &self.connection.runs
    }

    /// The running engine's Product and version.
    pub fn identity(&self) -> &Identity {
        &self.connection.identity
    }

    /// The log events of the call's Connection.
    pub(crate) fn log(&self) -> &ConnectionLog {
        &self.connection.log
    }

    /// The Daemon the call's Connection reaches, or `None` on an Attached
    /// engine.
    pub(crate) fn daemon(&self) -> Option<&Arc<dyn DaemonSide>> {
        self.connection.daemon.as_ref()
    }

    /// The person the call acts for, as the Audit log names them: the one
    /// the Console's sign-in driver signs in where the Daemon serves one,
    /// else the OS user, its own name on a single-user install (#826).
    pub(crate) fn person(&self) -> String {
        self.daemon()
            .and_then(|daemon| daemon.sign_ins())
            .map_or_else(
                || self.runs().person(),
                |sign_ins| sign_ins.person().to_owned(),
            )
    }

    /// Cancelled when the call is: a handler that waits watches it at each
    /// cancellation point.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.token
    }
}

type Answer = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

/// How a control's weakening is recorded: the Audit kind it records and
/// the built-in value it names as the default.
type Recorded = (&'static SharedTypeValue, fn() -> Value);

/// What drives a kind of Run, such as Foxhound's crawl's delivery, given
/// the Run, the context of the call that resumed it, whose Connection's
/// Client serves it, and its settings: a resume starts it again on the
/// Run's committed state.
pub type Driver = Arc<
    dyn Fn(Run, CallContext, RunSettings) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// A call's answer as a [`Guard`] gives it in place of its handler's: the
/// result, as JSON, or its error.
pub type GuardAnswer = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

/// What a contribution asks of each of its calls before its handler, given
/// the operation, the call's params as they arrived and its context: an
/// answer given in the handler's place, or `None` to let the handler
/// answer. It runs when the request arrives, as the handler does.
pub type Guard =
    Arc<dyn Fn(&'static Operation, &Value, &CallContext) -> Option<GuardAnswer> + Send + Sync>;

/// The handler of one operation of the catalog.
#[derive(Clone)]
pub struct Handler {
    operation: &'static Operation,
    call: Arc<dyn Fn(Value, CallContext) -> Answer + Send + Sync>,
    streams: bool,
    input: Option<(&'static str, usize)>,
}

impl Handler {
    /// Handles `operation` with `handle`, which takes the operation's params
    /// type and answers its result type; params that do not read as `P`,
    /// such as an array where a struct belongs, answer `invalid_params`.
    /// `handle` itself is called when the request arrives, before the
    /// Connection reads its next message, and its future runs as a task:
    /// what it does before returning its future, such as arming a route, is
    /// done in the order the Client sent its calls (ADR 0080).
    pub fn new<P, R, F, Fut>(operation: &'static Operation, handle: F) -> Self
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + 'static,
        F: Fn(P, CallContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, CallError>> + Send + 'static,
    {
        let call = move |params: Value, context: CallContext| -> Answer {
            // `handle` runs when the request arrives, before the next one
            // is read, so what it does before its future (arming a route or
            // a waiter, ADR 0080) is done in the Connection's arrival order.
            let answering = params::read::<P>(params)
                .map(|params| handle(params, context))
                .map_err(|error| {
                    CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
                });
            Box::pin(async move {
                let result = answering?.await?;
                serde_json::to_value(result).map_err(|error| {
                    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
                })
            })
        };
        Self {
            operation,
            call: Arc::new(call),
            streams: false,
            input: None,
        }
    }

    /// The same handler, whose calls stream notifications under the credits
    /// their Client grants; only its calls get credits and a stream.
    #[must_use]
    pub fn streaming(mut self) -> Self {
        self.streams = true;
        self
    }

    /// Whether its calls stream notifications.
    pub(crate) fn streams(&self) -> bool {
        self.streams
    }

    /// The same handler, whose calls take the notifications `method` that
    /// name them as their input, through [`CallContext::input`], at most
    /// `room` of them unread: the most its calls' credits let a Client
    /// send unanswered. A Client that sends one more fails the call's next
    /// read with `invalid_params`.
    #[must_use]
    pub fn taking_input(mut self, method: &'static str, room: usize) -> Self {
        self.input = Some((method, room));
        self
    }

    /// The notification its calls take as their input, with their room,
    /// if any.
    pub(crate) fn input(&self) -> Option<(&'static str, usize)> {
        self.input
    }

    /// The operation it handles.
    pub fn operation(&self) -> &'static Operation {
        self.operation
    }

    pub(crate) fn call(&self, params: Value, context: CallContext) -> Answer {
        (self.call)(params, context)
    }
}

/// What one domain crate adds to the engine.
#[derive(Clone)]
pub struct Contribution {
    name: &'static str,
    handlers: Vec<Handler>,
    run_state_tables: Vec<RunStateTables>,
    politeness_tables: Vec<PolitenessTables>,
    drivers: Vec<(RunKind, Driver)>,
    admission_gates: Option<Arc<dyn Gates>>,
    report_parts: Vec<Arc<dyn ReportPart>>,
    progress_parts: Vec<Arc<dyn ProgressPart>>,
    clean_up_versions: Option<Arc<dyn CleanUpVersions>>,
    needs_you_sources: Vec<NeedsYouSource>,
    controls: Vec<(&'static Setting, Option<Recorded>)>,
    secret_providers: Vec<(Provider, ProviderFactory)>,
}

impl Contribution {
    /// An empty contribution of the crate or concern `name`.
    pub fn new(name: &'static str) -> Self {
        Self {
            secret_providers: Vec::new(),
            name,
            handlers: Vec::new(),
            run_state_tables: Vec::new(),
            politeness_tables: Vec::new(),
            drivers: Vec::new(),
            admission_gates: None,
            report_parts: Vec::new(),
            progress_parts: Vec::new(),
            clean_up_versions: None,
            needs_you_sources: Vec::new(),
            controls: Vec::new(),
        }
    }

    /// The same, reporting `setting` as a switchable control (ADR 0016): a
    /// Run that starts with a value of it that weakens is recorded once in
    /// the Audit log, warned of once through the host's logging and lists
    /// it in its report's `weakened`, as the `security.*` keys are.
    #[must_use]
    pub fn with_control(mut self, setting: &'static Setting) -> Self {
        self.controls.push((setting, None));
        self
    }

    /// The same, recording a Run's value of `setting` that weakens as the
    /// Audit log kind `kind`, once per Run, with the value, its built-in
    /// default `built_in` and the layer that set it, warned of once through
    /// the host's logging and listed in its report's `weakened`: Foxhound's
    /// Politeness and Site-preferences values that press a site harder (ADR
    /// 0086).
    #[must_use]
    pub fn with_recorded_weakening(
        mut self,
        setting: &'static Setting,
        kind: &'static SharedTypeValue,
        built_in: fn() -> Value,
    ) -> Self {
        self.controls.push((setting, Some((kind, built_in))));
        self
    }

    /// The same, answering `provider`'s Secret references with the provider
    /// `factory` builds for each Run from its settings (ADR 0129), such as
    /// a built-in external provider configured by
    /// `secret_providers.<scheme>.*`.
    #[must_use]
    pub fn with_secret_provider(mut self, provider: Provider, factory: ProviderFactory) -> Self {
        self.secret_providers.push((provider, factory));
        self
    }

    /// The same, driving the Runs of `kind` it starts with `driver`, which a
    /// resume starts again (ADR 0102).
    #[must_use]
    pub fn with_driver(mut self, kind: RunKind, driver: Driver) -> Self {
        self.drivers.push((kind, driver));
        self
    }

    /// The same, adding `handler`.
    #[must_use]
    pub fn with_handler(mut self, handler: Handler) -> Self {
        self.handlers.push(handler);
        self
    }

    /// The same, each of its handlers' calls asking `guard` first, whose
    /// answer, when it gives one, the call takes in place of its handler's.
    #[must_use]
    pub fn guarded(mut self, guard: &Guard) -> Self {
        for handler in &mut self.handlers {
            let (inner, operation, guard) = (
                Arc::clone(&handler.call),
                handler.operation,
                Arc::clone(guard),
            );
            handler.call = Arc::new(move |params: Value, context: CallContext| {
                match guard(operation, &params, &context) {
                    Some(answer) => answer,
                    None => inner(params, context),
                }
            });
        }
        self
    }

    /// The same, adding `tables` to every run file (ADR 0087).
    #[must_use]
    pub fn with_run_state_tables(mut self, tables: RunStateTables) -> Self {
        self.run_state_tables.push(tables);
        self
    }

    /// The same, adding `tables` to the per-user Politeness file, where a
    /// rule of its gate in admission keeps its keys' shared state (ADR
    /// 0053).
    #[must_use]
    pub fn with_politeness_tables(mut self, tables: PolitenessTables) -> Self {
        self.politeness_tables.push(tables);
        self
    }

    /// The same, passing in `gates` as the gate in admission each Run opens
    /// (ADR 0087).
    #[must_use]
    pub fn with_admission_gates(mut self, gates: Arc<dyn Gates>) -> Self {
        self.admission_gates = Some(gates);
        self
    }

    /// The same, adding `part` to each Run's report when the Run ends (ADR
    /// 0102).
    #[must_use]
    pub fn with_report_part(mut self, part: Arc<dyn ReportPart>) -> Self {
        self.report_parts.push(part);
        self
    }

    /// The same, adding `part` to each value of a Run's progress, read from
    /// its committed state (ADR 0102).
    #[must_use]
    pub fn with_progress_part(mut self, part: Arc<dyn ProgressPart>) -> Self {
        self.progress_parts.push(part);
        self
    }

    /// The same, passing in `versions` as the Clean-up rules that choose
    /// the version each new Run holds to (ADR 0047).
    #[must_use]
    pub fn with_clean_up_versions(mut self, versions: Arc<dyn CleanUpVersions>) -> Self {
        self.clean_up_versions = Some(versions);
        self
    }

    /// The same, adding `source` to "Needs you" (spec #592): its items join
    /// the one list, whatever their kind, without the engine naming it
    /// (ADR 0087).
    #[must_use]
    pub fn with_needs_you_source(mut self, source: NeedsYouSource) -> Self {
        self.needs_you_sources.push(source);
        self
    }

    /// Who contributes it.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// One file of a Console build: its path from the build's root, such as
/// `/assets/index-4f2a.js`, and its bytes, brotli-compressed.
#[derive(Clone, Copy, Debug)]
pub struct ConsoleFile {
    pub(crate) path: &'static str,
    pub(crate) brotli: &'static [u8],
}

impl ConsoleFile {
    /// The file at `path`, such as `/index.html`, holding `brotli`.
    pub const fn new(path: &'static str, brotli: &'static [u8]) -> Self {
        Self { path, brotli }
    }
}

/// A Product's Console build, as its executable embeds it: brotli files
/// only, `xtask console build`'s output (ADR 0057), and the routes of its
/// screens, which its package commits as plain data (ADR 0072).
#[derive(Debug)]
pub struct ConsoleBuild {
    files: &'static [ConsoleFile],
    routes: &'static [&'static str],
}

impl ConsoleBuild {
    /// The build made of `files`, whose page shows each of `routes`, such
    /// as `/settings`, or `/runs/$run`, whose segment `$run` stands for
    /// any one segment.
    pub const fn new(files: &'static [ConsoleFile], routes: &'static [&'static str]) -> Self {
        Self { files, routes }
    }

    /// The file a request's path names: `/` and each route of the build
    /// name the page itself, which shows the route's screen.
    pub(crate) fn file(&self, path: &str) -> Option<&ConsoleFile> {
        let path = if path == "/" || self.routes.iter().any(|route| matches_route(route, path)) {
            "/index.html"
        } else {
            path
        };
        self.files.iter().find(|file| file.path == path)
    }
}

/// Whether `path` is one of `route`'s: the same segments, a segment of
/// `route` that begins with `$` standing for any one that is not empty.
fn matches_route(route: &str, path: &str) -> bool {
    let mut routes = route.split('/');
    let mut paths = path.split('/');
    loop {
        match (routes.next(), paths.next()) {
            (None, None) => return true,
            (Some(expected), Some(given)) => {
                let fits = if expected.starts_with('$') {
                    !given.is_empty()
                } else {
                    expected == given
                };
                if !fits {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// The contributions an engine starts with.
#[derive(Clone, Default)]
pub struct Composition {
    contributions: Vec<Contribution>,
    console: Option<&'static ConsoleBuild>,
}

/// A composition no engine can start from.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CompositionError {
    /// Two handlers for one operation.
    #[error("`{operation}` has a handler from both `{first}` and `{second}`")]
    DuplicateHandler {
        /// The operation.
        operation: &'static str,
        /// The first contribution that handles it.
        first: &'static str,
        /// The second.
        second: &'static str,
    },
    /// Two gates in admission.
    #[error("both `{first}` and `{second}` pass in a gate in admission")]
    DuplicateGates {
        /// The first contribution that passes one in.
        first: &'static str,
        /// The second.
        second: &'static str,
    },
    /// Two Clean-up rules.
    #[error("both `{first}` and `{second}` pass in the Clean-up rules")]
    DuplicateCleanUpVersions {
        /// The first contribution that passes them in.
        first: &'static str,
        /// The second.
        second: &'static str,
    },
}

impl Composition {
    /// The same, adding `contribution`.
    #[must_use]
    pub fn with(mut self, contribution: Contribution) -> Self {
        self.contributions.push(contribution);
        self
    }

    /// The same, serving `console` as its Daemon's Console: only an
    /// executable crate passes one (ADR 0031), so an SDK crate's linked
    /// engine serves none.
    #[must_use]
    pub fn with_console(mut self, console: &'static ConsoleBuild) -> Self {
        self.console = Some(console);
        self
    }

    /// The Console build it serves, if any.
    pub fn console(&self) -> Option<&'static ConsoleBuild> {
        self.console
    }

    /// The name of each contribution, in order.
    pub fn contributions(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.contributions.iter().map(Contribution::name)
    }

    /// The operation of each handler, once per handler, so a test sees an
    /// operation handled twice.
    pub fn operations(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.contributions
            .iter()
            .flat_map(|contribution| &contribution.handlers)
            .map(|handler| handler.operation.name)
    }

    /// The tables every contribution adds to every run file, in order.
    pub(crate) fn run_state_tables(&self) -> Vec<RunStateTables> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.run_state_tables.iter().copied())
            .collect()
    }

    /// The tables every contribution adds to the Politeness file, in order.
    pub(crate) fn politeness_tables(&self) -> Vec<PolitenessTables> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.politeness_tables.iter().copied())
            .collect()
    }

    /// The driver of each kind of Run, the last given for a kind.
    pub(crate) fn drivers(&self) -> BTreeMap<RunKind, Driver> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.drivers.iter().cloned())
            .collect()
    }

    /// What every contribution adds to each Run's report, in order.
    pub(crate) fn report_parts(&self) -> Vec<Arc<dyn ReportPart>> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.report_parts.iter().cloned())
            .collect()
    }

    /// What every contribution adds to each value of a Run's progress, in
    /// order.
    pub(crate) fn progress_parts(&self) -> Vec<Arc<dyn ProgressPart>> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.progress_parts.iter().cloned())
            .collect()
    }

    /// Every contribution's switchable controls.
    pub(crate) fn controls(&self) -> Controls {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.controls.iter().copied())
            .fold(
                Controls::default(),
                |controls, (setting, kind)| match kind {
                    Some((kind, built_in)) => controls.with_recorded(setting, kind, built_in),
                    None => controls.with(setting),
                },
            )
    }

    /// Every contribution's Secret providers built for each Run, in order:
    /// a later one for a scheme replaces an earlier.
    pub fn secret_providers(&self) -> Vec<(Provider, ProviderFactory)> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.secret_providers.iter().cloned())
            .collect()
    }

    /// Every contribution's sources of "Needs you", in order.
    pub(crate) fn needs_you_sources(&self) -> Vec<NeedsYouSource> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.needs_you_sources.iter().cloned())
            .collect()
    }

    /// The one gate in admission the contributions pass in, if any.
    pub(crate) fn admission_gates(&self) -> Result<Option<Arc<dyn Gates>>, CompositionError> {
        let mut passed = self.contributions.iter().filter_map(|contribution| {
            Some((contribution.name, contribution.admission_gates.clone()?))
        });
        let first = passed.next();
        if let (Some((first, _)), Some((second, _))) = (&first, passed.next()) {
            return Err(CompositionError::DuplicateGates { first, second });
        }
        Ok(first.map(|(_, gates)| gates))
    }

    /// The one Clean-up rules the contributions pass in, if any.
    pub(crate) fn clean_up_versions(
        &self,
    ) -> Result<Option<Arc<dyn CleanUpVersions>>, CompositionError> {
        let mut passed = self.contributions.iter().filter_map(|contribution| {
            Some((contribution.name, contribution.clean_up_versions.clone()?))
        });
        let first = passed.next();
        if let (Some((first, _)), Some((second, _))) = (&first, passed.next()) {
            return Err(CompositionError::DuplicateCleanUpVersions { first, second });
        }
        Ok(first.map(|(_, versions)| versions))
    }

    /// Each operation's one handler.
    pub(crate) fn handlers(&self) -> Result<BTreeMap<&'static str, Handler>, CompositionError> {
        let mut handlers: BTreeMap<&'static str, (&'static str, Handler)> = BTreeMap::new();
        for contribution in &self.contributions {
            for handler in &contribution.handlers {
                let operation = handler.operation.name;
                if let Some((first, _)) = handlers.get(operation) {
                    return Err(CompositionError::DuplicateHandler {
                        operation,
                        first,
                        second: contribution.name,
                    });
                }
                handlers.insert(operation, (contribution.name, handler.clone()));
            }
        }
        Ok(handlers
            .into_iter()
            .map(|(operation, (_, handler))| (operation, handler))
            .collect())
    }
}
