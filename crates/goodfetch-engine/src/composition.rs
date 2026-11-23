//! The composition (ADR 0087): each domain crate exports one
//! [`Contribution`] listing what it adds to the engine, and each Product's
//! executable crate passes the ones it links to the engine at start, a list
//! in its `main`. Nothing registers itself by being linked. A contribution
//! adds operation handlers and the drivers of its kinds of Run, the tables
//! it keeps in every run file and the Politeness file, its gates in
//! admission, its parts of a Run's report and progress, its Clean-up
//! versions, its send window, its "Needs you" sources, its Secret
//! providers and the controls and weakenings it records.
//!
//! A handler answers a value or a [`CallError`], which only the RPC layer
//! turns into the wire's error object.
//!
//! Size: one concern, what a contribution adds, each a field, its builder
//! and the composition's reader, kept side by side.

mod call_streams;
mod closing;
pub(crate) mod params;

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use goodfetch_protocol::catalog::{Operation, Setting, SharedTypeValue};
use goodfetch_protocol::connection::{Form, Surface};
use goodfetch_protocol::daemon::{DaemonHealth, DaemonState, DaemonStopping};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use tokio_util::sync::CancellationToken;

use crate::admission::Gates;
use crate::call_error::CallError;
use crate::client::Client;
use crate::controls::{Controls, Recorded};
use crate::engine_context::EngineContext;
use crate::events::Events;
use crate::log::ConnectionLog;
use crate::needs_you::NeedsYouSource;
use crate::politeness::PolitenessTables;
use crate::product::Identity;
use crate::run_state::RunStateTables;
use crate::runs::{CleanUpVersions, Driving, ProgressPart, ReportPart, Run, Runs, SendWindow};
use crate::secrets::ProviderFactory;
use crate::settings::RunSettings;
use goodfetch_protocol::run::{RunId, RunKind};
use goodfetch_protocol::secret_reference::Provider;

pub use call_streams::{CallInput, CallStream};
pub(crate) use call_streams::{CallNotifier, InputSender};
pub(crate) use closing::Closing;
pub use params::Params;

/// What a handler is given beside its params: its Connection's handles,
/// shared by every call of that Connection, its cancellation, and for a
/// streaming call its own stream of notifications. A call clones one handle
/// of its Connection, so its boxed answer stays small (ADR 0134's figures
/// per dispatched message).
#[derive(Clone)]
pub struct CallContext {
    connection: Arc<ConnectionContext>,
    operation: &'static Operation,
    id: RequestId,
    token: CancellationToken,
    channels: Option<Arc<CallChannels>>,
}

/// What only a streaming handler's or an input-taking handler's calls
/// carry, behind one handle, so every other call's context stays the size
/// it was with neither.
pub(crate) struct CallChannels {
    stream: Option<CallStream>,
    input: Option<CallInput>,
}

impl CallChannels {
    /// A call's `stream` when its handler streams, and `input` when it
    /// takes input.
    pub(crate) fn new(stream: Option<CallStream>, input: Option<CallInput>) -> Self {
        Self { stream, input }
    }
}

/// What every call of one Connection is given: the engine's identity, the
/// Connection's Client and the surface its `hello` named, the engine's
/// Runs, event stream and context, and on a Daemon the Daemon itself. The protocol
/// server builds it once per Connection and sets the surface at `hello`.
pub(crate) struct ConnectionContext {
    pub(crate) identity: Arc<Identity>,
    pub(crate) client: Client,
    pub(crate) surface: OnceLock<Surface>,
    /// The form its `hello` declared, or its surface's (ADR 0099); set
    /// with the surface.
    pub(crate) form: OnceLock<Form>,
    /// Whether its `hello` came from Goodfetch Cloud's gateway, which words
    /// `not_in_product` for it (ADR 0096); set with the surface.
    pub(crate) behind_gateway: OnceLock<bool>,
    /// The Connection's own settings, which its `hello` gave and the
    /// engine accepted: the run layer at the connection scope, beneath
    /// each Run's own (ADR 0083); set with the surface.
    pub(crate) settings: OnceLock<Map<String, Value>>,
    /// The run layer's `registered` values its `hello`'s SDK handlers give,
    /// beneath every other run-layer value (ADR 0104); set with the surface.
    pub(crate) registered: OnceLock<Map<String, Value>>,
    /// The driver of each kind of Run that has one, which a resume starts
    /// again.
    pub(crate) drivers: Arc<BTreeMap<RunKind, Driver>>,
    pub(crate) runs: Runs,
    pub(crate) events: Events,
    /// What the engine gives every call: its home, clock and settings, and
    /// the user's stores.
    pub(crate) engine: EngineContext,
    pub(crate) daemon: Option<Arc<dyn DaemonSide>>,
    /// Whether it reached an Exposed Daemon's address (ADR 0049).
    pub(crate) exposed: bool,
    /// The log events the Connection receives, at the level it asks for.
    pub(crate) log: ConnectionLog,
}

/// Whether a Client's Connection holds the Daemon it reaches (ADR 0141).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holding {
    /// It holds the Daemon off its idle stop, and off a handover to its
    /// major's pick.
    Holds,
    /// It only observes, holding off neither: a Kennel's, unless its
    /// `hello` asks to keep the Daemon alive.
    Observes,
}

/// A boxed future of a [`DaemonSide`]'s answer.
pub(crate) type Answering<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a Daemon gives each of its Connections (ADRs 0024 and 0098): the
/// token every `hello` carries, the count of its Clients, whether it hands
/// over on a refusal, its answers to `daemon.*`, and how it closes the
/// Connection.
pub(crate) trait DaemonSide: Send + Sync {
    /// The per-launch token.
    fn token(&self) -> &str;
    /// How the Daemon closes this Connection.
    fn closing(&self) -> &Closing;
    /// A Client of `surface` opened; it counts until the value drops,
    /// toward the idle stop and a handover too as `holding` says (ADR
    /// 0141).
    fn opened(&self, surface: Surface, holding: Holding) -> Box<dyn Send + Sync>;
    /// A `hello` is being refused by Product or version: whether the Daemon
    /// hands over to its major's pick on it, which it has then begun.
    fn refusing(&self) -> Answering<'_, bool>;
    /// What `daemon.status` answers.
    fn status(&self) -> Answering<'_, DaemonState>;
    /// What `daemon.health` answers.
    fn health(&self) -> Answering<'_, DaemonHealth>;
    /// Stops the Daemon once `daemon.stop` has answered what it says,
    /// closing its other Connections with `closing`, which their calls in
    /// flight raise.
    fn stop(&self, closing: CallError) -> Answering<'_, DaemonStopping>;
}

impl CallContext {
    /// The call `id` of `operation` on `connection`, cancelled through
    /// `token`, with `channels` when its handler streams or takes input.
    pub(crate) fn new(
        connection: Arc<ConnectionContext>,
        operation: &'static Operation,
        id: RequestId,
        token: CancellationToken,
        channels: Option<CallChannels>,
    ) -> Self {
        Self {
            connection,
            operation,
            id,
            token,
            channels: channels.map(Arc::new),
        }
    }

    /// The operation called, which an error of the call names.
    pub fn operation(&self) -> &'static Operation {
        self.operation
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
        self.channels
            .as_deref()
            .and_then(|channels| channels.input.as_ref())
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
        self.channels
            .as_deref()
            .and_then(|channels| channels.stream.as_ref())
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

    /// The form the Connection's Client writes names in, which the engine
    /// spells an operation's name in at the head of its errors (ADRs 0079
    /// and 0099).
    pub fn form(&self) -> Form {
        self.connection
            .form
            .get()
            .copied()
            .expect("invariant: a call runs only on a Connection whose `hello` was answered")
    }

    /// The Connection's own settings, nested by key, as its `hello` gave
    /// them: empty when it gave none (ADR 0083).
    pub fn connection_settings(&self) -> &Map<String, Value> {
        self.connection
            .settings
            .get()
            .expect("invariant: a call runs only on a Connection whose `hello` was answered")
    }

    /// The run layer's `registered` values the Connection's SDK handlers
    /// give, by dotted key: empty when none has one (ADR 0104).
    pub fn registered_settings(&self) -> &Map<String, Value> {
        self.connection
            .registered
            .get()
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

    /// Whether the Connection reached an Exposed Daemon's address (ADR
    /// 0049), where the project configuration a Client sends has only the
    /// run layer's authority (ADR 0083).
    pub fn exposed(&self) -> bool {
        self.connection.exposed
    }

    /// The driver of Runs of `kind`, when the composition has one.
    pub fn driver(&self, kind: RunKind) -> Option<Driver> {
        self.connection.drivers.get(&kind).cloned()
    }

    /// The engine's Runs.
    pub fn runs(&self) -> &Runs {
        &self.connection.runs
    }

    /// What the engine gives every call: its home, clock and settings, the
    /// user's Audit log, known-certificates store and Usage ledger, and
    /// where blocking work on them runs.
    pub fn engine(&self) -> &EngineContext {
        &self.connection.engine
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

    /// The person the call acts for, as the Audit log names them: the OS
    /// user, its own name on a single-user install (#826).
    pub(crate) fn person(&self) -> String {
        self.engine().person().to_owned()
    }

    /// Cancelled when the call is: a handler that waits watches it at each
    /// cancellation point.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.token
    }
}

type Answer = Pin<Box<dyn Future<Output = Result<Answered, CallError>> + Send>>;

/// A call's result, encoded once, straight into the line of its success
/// (ADR 0140's E1).
#[derive(Debug)]
pub struct Answered(Vec<u8>);

impl Answered {
    /// The success of the call `context` names, with `result`.
    ///
    /// # Errors
    ///
    /// `internal_error` when `result` does not encode as JSON, such as a
    /// map whose keys are no strings.
    pub fn encode(context: &CallContext, result: &impl Serialize) -> Result<Self, CallError> {
        Self::of(context.id(), result)
    }

    fn of(id: &RequestId, result: &impl Serialize) -> Result<Self, CallError> {
        crate::wire::success_line(id, result)
            .map(Self)
            .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))
    }

    /// Its line.
    pub(crate) fn into_line(self) -> Vec<u8> {
        self.0
    }
}

/// What drives a kind of Run, such as Foxhound's crawl's delivery, given
/// the Run, the context of the call that resumed it, whose Connection's
/// Client serves it, and its settings: a resume starts it again on the
/// Run's committed state.
pub type Driver = Arc<
    dyn Fn(Run, CallContext, RunSettings) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// What finds the Run a call names by its page or its Session, IDs only
/// the contribution that makes them reads: the Run whose trace records a
/// raw call (ADR 0099) that names no Run ID.
pub trait RunOfCall: Send + Sync {
    /// The Run of the page `page_id`, open or crashed, if it is one of
    /// this contribution's.
    fn page_run<'a>(
        &'a self,
        page_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<RunId>> + Send + 'a>>;

    /// The Run of the Session `session_id`, if it is one of this
    /// contribution's.
    fn session_run(&self, session_id: &str) -> Option<RunId>;
}

/// A call's answer as a [`Guard`] gives it in place of its handler's: the
/// result, encoded, or its error.
pub type GuardAnswer = Answer;

/// What a contribution asks of each of its calls before its handler, given
/// the operation, the call's params as they arrived and as its handler's
/// type read them, and its context: an answer given in the handler's
/// place, or `None` to let the handler answer. It runs when the request
/// arrives, as the handler does.
pub type Guard = Arc<
    dyn for<'params, 'read> Fn(
            &'static Operation,
            Params<'params>,
            ReadParams<'read>,
            &CallContext,
        ) -> Option<GuardAnswer>
        + Send
        + Sync,
>;

/// What ends a contribution's calls at the end of an enclosing call (ADR
/// 0080), given the operation, the call's params as they arrived, its
/// context and its answer: that answer, ended once the enclosing call's
/// end passes first, such as a page call's in a crawl handler under
/// `timeout.handler`, or as it was when no call encloses it. It runs when
/// the request arrives, once the guards or the handler gave the answer,
/// and only on a cancellable operation, whose answer may be dropped.
#[derive(Clone)]
pub struct Enclose(Arc<EncloseFn>);

/// The function an [`Enclose`] holds.
type EncloseFn = dyn for<'params> Fn(&'static Operation, Params<'params>, &CallContext, GuardAnswer) -> GuardAnswer
    + Send
    + Sync;

impl Enclose {
    /// The one that ends each answer as `enclose` does.
    pub fn new(
        enclose: impl for<'params> Fn(
            &'static Operation,
            Params<'params>,
            &CallContext,
            GuardAnswer,
        ) -> GuardAnswer
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self(Arc::new(enclose))
    }
}

/// A call's params as its handler's type read them, when they read: what a
/// [`Guard`] answers from in the handler's place, so that a call decodes
/// its params once whoever answers it (ADR 0140's E1).
#[derive(Clone, Copy)]
pub struct ReadParams<'read>(Option<&'read dyn Any>);

impl<'read> ReadParams<'read> {
    /// The params as `P`: `None` when they did not read into the handler's
    /// type or its type is not `P`.
    #[must_use]
    pub fn get<P: 'static>(self) -> Option<&'read P> {
        self.0?.downcast_ref()
    }
}

/// A handler's call, given the call's params, its context and the guards
/// to ask: the read of the params into the handler's type, in whose pass
/// dispatch finds what it takes of them, then each guard, then the handler.
/// It fails, before any guard is asked, only with what refuses the request
/// before its call starts: a `_meta` dispatch does not take.
type Call = dyn for<'params> Fn(Params<'params>, CallContext, &[Guard]) -> Result<Answer, CallError>
    + Send
    + Sync;

/// Each operation's one handler, found by its name on every message.
///
/// A hash finds a name in one probe however many operations the
/// contributions add, where an ordered map compared it with a key on each
/// level of its tree, and foldhash hashes a short name in a few
/// multiplications where std's `SipHash` takes rounds (issue 2790). Its seed
/// is fixed: a random one would change a lookup's work from one process to
/// the next (randomness outside the Entropy handle, ADR 0059). Only the
/// composition inserts, so a Client's method name lengthens no probe.
/// Nothing iterates it.
pub(crate) type Handlers = HashMap<&'static str, Handler, foldhash::fast::FixedState>;

/// The handler of one operation of the catalog.
#[derive(Clone)]
pub struct Handler {
    operation: &'static Operation,
    call: Arc<Call>,
    /// The guards its calls ask before it, the last added first.
    guards: Arc<[Guard]>,
    /// What ends its calls' answers at an enclosing call's end.
    encloses: Arc<[Enclose]>,
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
        let call = move |params: Params<'_>,
                         context: CallContext,
                         guards: &[Guard]|
              -> Result<Answer, CallError> {
            // The one read of the params (ADR 0140's E1): into `P`, and in
            // the same pass what dispatch takes of them. It changes
            // nothing, so the order of refusals stays: a `_meta` dispatch
            // refuses, then a guard's answer, then params that are no `P`.
            let read = params.read::<P>();
            params.taken()?;
            let typed = ReadParams(read.as_ref().ok().map(|params| params as &dyn Any));
            for guard in guards.iter().rev() {
                if let Some(answer) = guard(operation, params, typed, &context) {
                    return Ok(answer);
                }
            }
            // `handle` runs when the request arrives, before the next one
            // is read, so what it does before its future (arming a route or
            // a waiter, ADR 0080) is done in the Connection's arrival order.
            let answering = read.map(|params| {
                let id = context.id().clone();
                (id, handle(params, context))
            });
            Ok(Box::pin(async move {
                let (id, answering) = answering?;
                let result = answering.await?;
                Answered::of(&id, &result)
            }))
        };
        Self {
            operation,
            call: Arc::new(call),
            guards: Arc::new([]),
            encloses: Arc::new([]),
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

    /// Starts its call on `params`: the guards it asks, then the handler,
    /// which reads `params` before this returns, its answer ended at an
    /// enclosing call's end when its operation is cancellable.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a `_meta` dispatch refuses: the call does not
    /// start, and neither a guard nor the handler ran.
    pub(crate) fn call(
        &self,
        params: Params<'_>,
        context: CallContext,
    ) -> Result<Answer, CallError> {
        if self.encloses.is_empty() || !self.operation.cancellable {
            return (self.call)(params, context, &self.guards);
        }
        let enclosed = context.clone();
        let answer = (self.call)(params, context, &self.guards)?;
        Ok(self.encloses.iter().fold(answer, |answer, enclose| {
            (enclose.0)(self.operation, params, &enclosed, answer)
        }))
    }
}

/// What one domain crate adds to the engine.
#[derive(Clone)]
pub struct Contribution {
    name: &'static str,
    handlers: Vec<Handler>,
    run_state_tables: Vec<RunStateTables>,
    politeness_tables: Vec<PolitenessTables>,
    drivers: Vec<(RunKind, Driver, Driving)>,
    admission_gates: Option<Arc<dyn Gates>>,
    report_parts: Vec<Arc<dyn ReportPart>>,
    progress_parts: Vec<Arc<dyn ProgressPart>>,
    clean_up_versions: Option<Arc<dyn CleanUpVersions>>,
    send_window: Option<Arc<dyn SendWindow>>,
    needs_you_sources: Vec<NeedsYouSource>,
    controls: Vec<(&'static Setting, Recorded)>,
    secret_providers: Vec<(Provider, ProviderFactory)>,
    runs_of_calls: Vec<Arc<dyn RunOfCall>>,
}

impl Contribution {
    /// An empty contribution of the crate or concern `name`.
    pub fn new(name: &'static str) -> Self {
        Self {
            secret_providers: Vec::new(),
            runs_of_calls: Vec::new(),
            name,
            handlers: Vec::new(),
            run_state_tables: Vec::new(),
            politeness_tables: Vec::new(),
            drivers: Vec::new(),
            admission_gates: None,
            report_parts: Vec::new(),
            progress_parts: Vec::new(),
            clean_up_versions: None,
            send_window: None,
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
        self.controls.push((setting, Recorded::Switch));
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
        self.controls
            .push((setting, Recorded::As { kind, built_in }));
        self
    }

    /// The same, recording a Run's value of `setting` that lists anything
    /// as the Audit log kind `kind`, once per Run, always chained, with
    /// what it lists under `field` and the layer that set it, and listed in
    /// its report's `weakened`: an allowance of what the engine refuses by
    /// default, such as `egress.private_ranges` (ADR 0016).
    #[must_use]
    pub fn with_recorded_listing(
        mut self,
        setting: &'static Setting,
        kind: &'static SharedTypeValue,
        field: &'static str,
    ) -> Self {
        self.controls
            .push((setting, Recorded::Listing { kind, field }));
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
    /// resume starts again (ADR 0102), and saying how such a Run goes with
    /// `driving`: who ends it, how its start commits and which setting says
    /// whether it keeps a Provenance log (ADR 0099).
    #[must_use]
    pub fn with_driver(mut self, kind: RunKind, driver: Driver, driving: Driving) -> Self {
        self.drivers.push((kind, driver, driving));
        self
    }

    /// The same, finding with `runs` the Run a call names by one of its
    /// pages or Sessions, whose trace records a raw call (ADR 0099).
    #[must_use]
    pub fn with_run_of_call(mut self, runs: Arc<dyn RunOfCall>) -> Self {
        self.runs_of_calls.push(runs);
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
            handler.guards = handler
                .guards
                .iter()
                .chain([guard])
                .map(Arc::clone)
                .collect();
        }
        self
    }

    /// The same, each of its handlers' cancellable calls ended by
    /// `enclose` at an enclosing call's end (ADR 0080).
    #[must_use]
    pub fn enclosed(mut self, enclose: &Enclose) -> Self {
        for handler in &mut self.handlers {
            handler.encloses = handler.encloses.iter().chain([enclose]).cloned().collect();
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

    /// The same, passing in `window` as the send window each Run's
    /// commands mark (ADR 0106): the engine's declared Effects.
    #[must_use]
    pub(crate) fn with_send_window(mut self, window: Arc<dyn SendWindow>) -> Self {
        self.send_window = Some(window);
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

/// The contributions an engine starts with.
#[derive(Clone, Default)]
pub struct Composition {
    contributions: Vec<Contribution>,
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
            .flat_map(|contribution| &contribution.drivers)
            .map(|(kind, driver, _)| (*kind, Arc::clone(driver)))
            .collect()
    }

    /// How the Runs of each kind go, as the last driver given for the kind
    /// says.
    pub(crate) fn driving(&self) -> BTreeMap<RunKind, Driving> {
        self.contributions
            .iter()
            .flat_map(|contribution| &contribution.drivers)
            .map(|(kind, _, driving)| (*kind, *driving))
            .collect()
    }

    /// What every contribution adds to each Run's report, in order.
    pub(crate) fn report_parts(&self) -> Vec<Arc<dyn ReportPart>> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.report_parts.iter().cloned())
            .collect()
    }

    /// What finds the Run a call names by a page or a Session, each
    /// contribution's in order.
    pub(crate) fn runs_of_calls(&self) -> Arc<[Arc<dyn RunOfCall>]> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.runs_of_calls.iter().cloned())
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
            .fold(Controls::default(), |controls, (setting, recorded)| {
                controls.with(setting, recorded)
            })
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

    /// The send window the engine's contribution, held once, passes in.
    pub(crate) fn send_window(&self) -> Option<Arc<dyn SendWindow>> {
        self.contributions
            .iter()
            .find_map(|each| each.send_window.clone())
    }

    /// Each operation's one handler.
    pub(crate) fn handlers(&self) -> Result<Handlers, CompositionError> {
        let mut handlers = Handlers::default();
        let mut owners: BTreeMap<&'static str, &'static str> = BTreeMap::new();
        for contribution in &self.contributions {
            for handler in &contribution.handlers {
                let operation = handler.operation.name;
                if let Some(first) = owners.insert(operation, contribution.name) {
                    return Err(CompositionError::DuplicateHandler {
                        operation,
                        first,
                        second: contribution.name,
                    });
                }
                handlers.insert(operation, handler.clone());
            }
        }
        Ok(handlers)
    }
}
