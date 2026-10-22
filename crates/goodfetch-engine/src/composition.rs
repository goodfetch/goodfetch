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

mod params;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use goodfetch_protocol::catalog::{ErrorCodeDefinition, Operation};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::daemon::{DaemonHealth, DaemonStatus, DaemonStopping};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::admission::Gates;
use crate::client::Client;
use crate::events::{Credits, Events};
use crate::log::ConnectionLog;
use crate::politeness::PolitenessTables;
use crate::product::Identity;
use crate::run_state::RunStateTables;
use crate::runs::{Driver, ReportPart, Runs};
use goodfetch_protocol::run::RunKind;

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

    /// What it says, for people.
    pub fn message(&self) -> &str {
        match &*self.0 {
            Failure::Code { message, .. } | Failure::Protocol { message, .. } => message,
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
    fn status(&self) -> Answering<'_, DaemonStatus>;
    /// What `daemon.health` answers.
    fn health(&self) -> Answering<'_, DaemonHealth>;
    /// Stops the Daemon once `daemon.stop` has answered what it says.
    fn stop(&self) -> Answering<'_, DaemonStopping>;
}

/// What a streaming call sends its notifications with: its id, the credits
/// its Client grants it and the Connection's writer.
pub(crate) struct CallStream {
    pub(crate) id: RequestId,
    pub(crate) credits: Credits,
    pub(crate) outgoing: mpsc::Sender<Vec<u8>>,
}

/// The stream of input a call takes from its Client (ADR 0024): each
/// notification of its handler's input method that names the call's id, in
/// the order they came, and the way back to its Client for what it
/// answers, such as the credits that let the Client send more.
pub struct CallInput {
    id: RequestId,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<Value>>,
    outgoing: mpsc::Sender<Vec<u8>>,
    line: fn(&str, &Value) -> Vec<u8>,
}

impl CallInput {
    /// The input of the call `id`, whose notifications `incoming` brings,
    /// answering on the Connection's writer `outgoing` with the lines
    /// `line` writes of a notification's method and params.
    pub(crate) fn new(
        id: RequestId,
        incoming: mpsc::UnboundedReceiver<Value>,
        outgoing: mpsc::Sender<Vec<u8>>,
        line: fn(&str, &Value) -> Vec<u8>,
    ) -> Self {
        Self {
            id,
            incoming: tokio::sync::Mutex::new(incoming),
            outgoing,
            line,
        }
    }

    /// The call's id, which its notifications name.
    pub fn id(&self) -> &RequestId {
        &self.id
    }

    /// The params of the next input notification; `None` once the
    /// Connection ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no notification.
    pub async fn next(&self) -> Option<Value> {
        self.incoming.lock().await.recv().await
    }

    /// Sends the Client the notification `method` with `params`; `false`
    /// once the Connection's writer is gone.
    pub async fn notify(&self, method: &str, params: &impl Serialize) -> bool {
        let params =
            serde_json::to_value(params).expect("invariant: a notification's params encode");
        self.outgoing
            .send((self.line)(method, &params))
            .await
            .is_ok()
    }
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
    pub(crate) fn stream(&self) -> &CallStream {
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

    /// Cancelled when the call is: a handler that waits watches it at each
    /// cancellation point.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.token
    }
}

type Answer = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

/// The handler of one operation of the catalog.
#[derive(Clone)]
pub struct Handler {
    operation: &'static Operation,
    call: Arc<dyn Fn(Value, CallContext) -> Answer + Send + Sync>,
    streams: bool,
    input: Option<&'static str>,
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
    pub(crate) fn streaming(mut self) -> Self {
        self.streams = true;
        self
    }

    /// Whether its calls stream notifications.
    pub(crate) fn streams(&self) -> bool {
        self.streams
    }

    /// The same handler, whose calls take the notifications `method` that
    /// name them as their input, through [`CallContext::input`].
    #[must_use]
    pub fn taking_input(mut self, method: &'static str) -> Self {
        self.input = Some(method);
        self
    }

    /// The notification its calls take as their input, if any.
    pub(crate) fn input(&self) -> Option<&'static str> {
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
}

impl Contribution {
    /// An empty contribution of the crate or concern `name`.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            handlers: Vec::new(),
            run_state_tables: Vec::new(),
            politeness_tables: Vec::new(),
            drivers: Vec::new(),
            admission_gates: None,
            report_parts: Vec::new(),
        }
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
/// only, `xtask console build`'s output (ADR 0057).
#[derive(Debug)]
pub struct ConsoleBuild {
    files: &'static [ConsoleFile],
}

impl ConsoleBuild {
    /// The build made of `files`.
    pub const fn new(files: &'static [ConsoleFile]) -> Self {
        Self { files }
    }

    /// The file a request's path names: `/` names the page itself.
    pub(crate) fn file(&self, path: &str) -> Option<&ConsoleFile> {
        let path = if path == "/" { "/index.html" } else { path };
        self.files.iter().find(|file| file.path == path)
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
