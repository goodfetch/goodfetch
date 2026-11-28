//! A process Plugin's instance at run time (ADRs 0033 and 0128): one task
//! of the owner it starts on, which starts the Plugin confined
//! ([`process`](super::process)), sends `hello` and holds the answer to the
//! approved manifest, queues the calls made before it is ready, then sends
//! each call and delivers its answer. Its state changes only through
//! [`lifecycle::apply`], and it carries out the effects the table gives.
//!
//! Its owner sets its scope. Started on a Run's owner, which every end of
//! the Run stops (it stops or finishes, and a Run is discarded only once
//! ended), the instance belongs to that Run and is killed with it. Started
//! on a task's own owner, outside any Run's life, it is killed when that
//! task ends. Every start runs a process of its own, so a task's instance
//! shares no process with a Run. Stopping it, or dropping every handle,
//! kills it too.
//!
//! The Plugin calls back only while it serves a call the instance sent it,
//! naming that call's id as its `call_id` (ADR 0033, story 7-53): its
//! `net.fetch` runs as a task of the instance's owner, under the checks of
//! [`plugin_gate`](crate::plugin_gate), and its answer goes back on the
//! Plugin's stdin. A call back that names no call the instance awaits is
//! refused with `plugin_call_back_refused`, and one still running when its
//! call is answered or the instance stops is cancelled, answered with
//! `call_cancelled`. At most [`CALL_BACKS`](crate::plugin_gate::CALL_BACKS)
//! run at once; a method the
//! engine does not serve is answered `method_not_found`.
//!
//! A message from the Plugin that answers nothing the instance awaits (a
//! second answer to `hello`, an answer to no call, a notification) is
//! decoded before the transition function and handled as a malformed
//! answer, logged at `warn` with `plugin_instance`: never an event.
//!
//! Each call is encoded once, by its caller, into its line. Each answer is
//! decoded once by `rpc`'s framing, and its caller reads the result from
//! where it is in that line (ADR 0140's E1).
//!
//! Crashes, restarts, quarantine and the heartbeat come with #837. Until
//! then, a process that ends ends its instance: its calls fail with
//! `plugin_crashed`, or with `confinement_unavailable` when its launcher
//! found that the host cannot confine it.
//!
//! Size: one instance state machine and its actor loop — process and
//! in-memory transports, FIFO concurrency credits, deadlines, `$/cancel`,
//! late-result refusal, uncertain Effect reporting and `net.fetch` call
//! backs all mutate the same [`Instance`] state in one loop.

#![expect(
    clippy::disallowed_types,
    reason = "the next call's id is taken in one step, never across an `.await` or under a lock, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use goodfetch_confinement::launch::Launch;
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::{ErrorCodeDefinition, OperationClass, Product};
use goodfetch_protocol::connection::{CALL_CANCELLED, INTERNAL, WaitedFor};
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::jsonrpc::{ErrorObject, ProtocolFailure, RequestId};
use goodfetch_protocol::plugin::net::{
    NET_CONNECT, NET_FETCH, NET_STREAM_CLOSE, NET_STREAM_CREDITS, NET_STREAM_DATA,
    NetConnectParams, NetConnectResult, NetFetchParams, NetFetchResult, NetStreamCloseParams,
    NetStreamCreditsParams, NetStreamDataParams, PLUGIN_CALL_BACK_REFUSED,
};
use goodfetch_protocol::plugin::spend::{USAGE_REPORT, UsageReportParams, UsageReportResult};
use goodfetch_protocol::plugin::{
    CANCEL, CONFINEMENT_UNAVAILABLE, HELLO, HelloRefusal, MISSED_PINGS, PING, PING_INTERVAL,
    PLUGIN_CRASHED, PLUGIN_LIMIT_EXCEEDED, PLUGIN_MANIFEST_MISMATCH, PLUGIN_PROTOCOL,
    PLUGIN_PROTOCOL_UNSUPPORTED, PLUGIN_QUARANTINED, PLUGIN_QUARANTINED_AUDIT, PluginCancelParams,
    PluginHelloParams, PluginHelloResult, PluginKindName, PluginLimitExceededDetails,
    PluginManifest, PluginPingParams, PluginResourceLimit, check_hello,
};
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::lifecycle::{
    self, CrashDecision, CrashTracker, InstanceEffect, InstanceEvent, InstanceState,
    InvalidInstanceTransition, RecentCrashes,
};
use super::limits::{self, ResourceMonitor};
use super::process::{Ended, Pipes, PluginProcess};
use super::spend::{DeclaredSpend, PluginSpend, ReportedSpend};
use super::streams::{INITIAL_CREDITS, STREAM_EVENTS, StreamEvent, StreamRegistry};
use crate::audit::{self, AuditLog};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::deadline::{Deadline, Enclosing};
use crate::framing::{self, Frame, Framer, Incoming, MAX_MESSAGE_BYTES, decode};
use crate::lease::LeaseCheck;
use crate::plugin_gate::{CALL_BACKS, OpenStreams, PluginFetch, admit_call_back};
use crate::tasks::TaskOwner;
use crate::transition::Refusal;
use crate::wire::{self, Received};

/// How often an active Plugin instance samples its resource monitor when one
/// is attached.
const RESOURCE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The id of the engine's `hello`: each call's id is one of the next.
const HELLO_ID: i64 = 0;

/// How many calls its mailbox holds before a caller waits. The manifest's
/// concurrency limit and its credits bound what the Plugin holds (story
/// 7-32); this only bounds what waits to be sent.
const MAILBOX: usize = 64;

/// How many bytes it reads from the Plugin at once.
const READ_BYTES: usize = 64 * 1024;

/// The most abandoned call ids kept for late-result refusal so a long-lived
/// instance never grows without bound.
const MAX_ABANDONED: usize = 1024;

/// What starts a process Plugin's instance.
#[derive(Debug)]
pub struct Plugin {
    /// The Product executable, whose `plugin-shim` mode is the launcher.
    pub launcher: PathBuf,
    /// The confined start: the policy, the Plugin's program and its
    /// private temporary folder, which must exist.
    pub launch: Launch,
    /// The approved manifest that its `hello` is held to.
    pub manifest: PluginManifest,
    /// The Product whose executable runs it, which `hello` names.
    pub product: Product,
    /// What its `net.fetch` reaches, and how.
    pub fetch: Arc<PluginFetch>,
    /// The clock its calls' deadlines wait on.
    pub clock: Clock,
    /// Optional Run spend accounting for the Plugin's declared `spend` units.
    pub spend: Option<PluginSpend>,
}

/// What starts an in-memory Plugin instance, over duplex streams speaking
/// the plugin protocol, for testing the host's rules on `Clock::paused`.
#[derive(Debug)]
pub struct InMemoryPlugin {
    /// The approved manifest that its `hello` is held to.
    pub manifest: PluginManifest,
    /// The Product whose engine runs it, which `hello` names.
    pub product: Product,
    /// What its `net.fetch` reaches, and how.
    pub fetch: Arc<PluginFetch>,
    /// The clock its calls' deadlines wait on.
    pub clock: Clock,
    /// Optional Run spend accounting for the Plugin's declared `spend` units.
    pub spend: Option<PluginSpend>,
}

/// The Plugin's end of an in-memory instance's pipes: what the engine writes
/// arrives on `from_engine`, and what the Plugin writes on `to_engine`
/// arrives on the engine's reader.
#[derive(Debug)]
pub struct InMemoryPeer {
    /// Bytes written by the engine (the Plugin's stdin).
    pub from_engine: DuplexStream,
    /// Bytes read by the engine (the Plugin's stdout).
    pub to_engine: DuplexStream,
}

/// An Effect-class Plugin call that was abandoned once sent: it may still
/// land, so it is reported uncertain at once for Reconciliation (ADRs 0029
/// and 0033).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncertainCall {
    /// The Plugin's name.
    pub plugin: String,
    /// The JSON-RPC id of the abandoned call.
    pub call_id: RequestId,
    /// The method that was sent.
    pub method: String,
    /// Why it is uncertain.
    pub reason: UncertainReason,
}

/// Receives each abandoned Effect-class call as soon as the instance
/// abandons it (ADRs 0029 and 0033).
pub trait UncertainReporter: fmt::Debug + Send + Sync {
    /// Reports that the Effect-class call `call` was abandoned in flight
    /// and is now uncertain.
    fn report(&self, call: UncertainCall);
}

/// Emitted once when a Plugin instance is quarantined for the rest of its run
/// after three crashes within 10 minutes (ADRs 0027 and 0033, issue 837).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuarantineEvent {
    /// The quarantined Plugin's name.
    pub plugin: String,
    /// The Run it belonged to, if any.
    pub run_id: Option<RunId>,
    /// The Product whose engine ran it.
    pub product: Product,
    /// Total crashes recorded by this instance, the quarantining one included.
    pub crashes: u32,
    /// Error code of the final crash (`plugin_crashed` or `plugin_limit_exceeded`).
    pub reason: String,
    /// Wall-clock timestamp of the quarantine.
    pub occurred_at: SystemTime,
}

/// Delivers the "a Plugin quarantined" Notification once per Run and Plugin
/// through ADR 0027's delivery ledger (issue 837).
pub trait QuarantineNotifier: fmt::Debug + Send + Sync {
    /// Delivers the Notification for `event`.
    fn notify(&self, event: QuarantineEvent) -> BoxFuture<'static, ()>;
}

/// Optional host observers and limit monitors attached when starting a
/// [`PluginInstance`] (ADR 0033, issue 837).
#[derive(Clone, Debug, Default)]
pub struct InstanceHooks {
    /// Audit log that records `plugin_quarantined` entries.
    pub audit: Option<AuditLog>,
    /// Notification sink invoked once when the instance is quarantined.
    pub quarantine_notifier: Option<Arc<dyn QuarantineNotifier>>,
    /// Explicit resource monitor overriding the default OS `/proc` sampler.
    pub resource_monitor: Option<Arc<dyn ResourceMonitor>>,
    /// Active `proxy_tcp` open stream counter for abandoned-call restart checks.
    pub proxied_streams: Option<OpenStreams>,
}

/// Executes a `page.*` or `locator.*` call back made by a process-Plugin Hook
/// while its `hook.call` is in flight (ADRs 0033, 0088 and 0104; issue 954).
pub trait PageCallBack: fmt::Debug + Send + Sync {
    /// Serves `method` with its JSON `params` (already admitted under its
    /// active `call_id`), returning the JSON result or a `CallError`.
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, CallError>>;
}

/// How a call into a [`PluginInstance`] is classed, timed and bounded
/// (ADRs 0029, 0033 and 0080).
#[derive(Clone, Debug)]
pub struct CallOptions {
    /// The class the engine gives the method: an abandoned `Effect` call
    /// is reported uncertain at once.
    pub class: OperationClass,
    /// The call's own deadline (`timeout.plugin`, `timeout.hook`, or
    /// `timeout.model`), if any.
    pub deadline: Option<Deadline>,
    /// The enclosing call's end (for example, a work item's lease), which
    /// the call's deadline never outlives.
    pub enclosing: Option<Enclosing>,
    /// The work item's lease, checked before sending and supplying its
    /// [`LeaseCheck::enclosing`] when `enclosing` is `None`.
    pub lease: Option<Arc<dyn LeaseCheck>>,
    /// Where an abandoned Effect-class call is reported at once.
    pub uncertain: Option<Arc<dyn UncertainReporter>>,
    /// Handler for `page.*` / `locator.*` calls back made while this call is
    /// in flight (only set for page-stage Hook calls).
    pub page_callback: Option<Arc<dyn PageCallBack>>,
}

impl Default for CallOptions {
    fn default() -> Self {
        Self {
            class: OperationClass::Read,
            deadline: None,
            enclosing: None,
            lease: None,
            uncertain: None,
            page_callback: None,
        }
    }
}

impl CallOptions {
    /// A read-class call waiting under `deadline`.
    #[must_use]
    pub fn read(deadline: Deadline) -> Self {
        Self {
            class: OperationClass::Read,
            deadline: Some(deadline),
            ..Self::default()
        }
    }

    /// An Effect-class call waiting under `deadline`, reported to
    /// `uncertain` at once if abandoned in flight.
    #[must_use]
    pub fn effect(deadline: Deadline, uncertain: Arc<dyn UncertainReporter>) -> Self {
        Self {
            class: OperationClass::Effect,
            deadline: Some(deadline),
            uncertain: Some(uncertain),
            ..Self::default()
        }
    }

    /// Bounds the call by an enclosing call's end, such as a work item's
    /// lease.
    #[must_use]
    pub fn with_enclosing(mut self, enclosing: Enclosing) -> Self {
        self.enclosing = Some(enclosing);
        self
    }

    /// Bounds the call by a work item's lease: refuses it if the lease has
    /// already ended, and never outlives [`LeaseCheck::enclosing`].
    #[must_use]
    pub fn with_lease(mut self, lease: Arc<dyn LeaseCheck>) -> Self {
        if self.enclosing.is_none() {
            self.enclosing = lease.enclosing();
        }
        self.lease = Some(lease);
        self
    }

    /// Reports an abandoned Effect-class call to `uncertain` at once.
    #[must_use]
    pub fn with_uncertain(mut self, uncertain: Arc<dyn UncertainReporter>) -> Self {
        self.uncertain = Some(uncertain);
        self
    }

    /// Serves `page.*` / `locator.*` calls back while this call is in flight.
    #[must_use]
    pub fn with_page_callback(mut self, callback: Arc<dyn PageCallBack>) -> Self {
        self.page_callback = Some(callback);
        self
    }
}

/// Whose work an instance serves, which its `hello` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// One Run's work. Start the instance on that Run's owner.
    Run(RunId),
    /// One task's work outside a Run's life, such as a probe or `plugins
    /// test`, or a finished Run's when it serves one, such as its backlog.
    /// Start the instance on the task's own owner.
    Task(Option<RunId>),
}

impl Scope {
    /// The Run its `hello` names.
    fn run_id(&self) -> Option<&RunId> {
        match self {
            Self::Run(run_id) => Some(run_id),
            Self::Task(run_id) => run_id.as_ref(),
        }
    }

    /// The event its owner's cancellation is.
    fn ended(&self) -> InstanceEvent {
        match self {
            Self::Run(_) => InstanceEvent::RunEnded,
            Self::Task(_) => InstanceEvent::Stop,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(run_id) => write!(formatter, "run {run_id}"),
            Self::Task(Some(run_id)) => write!(formatter, "task for run {run_id}"),
            Self::Task(None) => formatter.write_str("task"),
        }
    }
}

/// Why a call into an instance failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CallFailure {
    /// The Plugin answered with this error, which its kind's driver checks
    /// as any decoded input (ADR 0127).
    #[error("the Plugin answered: {}", .0.message)]
    Answered(ErrorObject),
    /// The Plugin's answer is no message the engine reads, which its kind's
    /// driver turns into the kind's `<kind>_handler_failed` (ADR 0127).
    #[error("the Plugin's answer cannot be read")]
    Malformed(Malformed),
    /// The call's deadline or enclosing lease passed before its answer
    /// came: the call was abandoned with `$/cancel` and fails with
    /// `deadline_exceeded` naming that setting (ADRs 0033 and 0080).
    #[error(transparent)]
    DeadlineExceeded(CallError),
    /// The instance did not carry the call out: refused at `hello`,
    /// crashed, quarantined, or stopped with its Run or its task.
    #[error(transparent)]
    Unavailable(CallError),
    /// The Run's Usage or spend rules refused the call (`limit_reached`,
    /// `limit_not_understood`, `plugin_price_unknown`).
    #[error(transparent)]
    Refused(CallError),
}

/// What made a Plugin's answer unreadable, for people and the log: never
/// matched on.
#[derive(Clone, Debug)]
pub struct Malformed {
    detail: String,
}

impl fmt::Display for Malformed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

/// A started instance, which every clone calls: the last one dropped stops
/// it.
#[derive(Clone, Debug)]
pub struct PluginInstance {
    calls: mpsc::Sender<Call>,
    cancels: mpsc::UnboundedSender<Cancel>,
    next: Arc<AtomicI64>,
    clock: Clock,
    spend: Option<DeclaredSpend>,
    stopping: CancellationToken,
    stop_reason: Arc<std::sync::Mutex<Option<CallError>>>,
    ended: watch::Receiver<Option<CallError>>,
}

/// A call on its way to the Plugin: its line, encoded once, its class and
/// uncertain reporter, and where its answer goes.
struct Call {
    id: RequestId,
    method: String,
    line: Vec<u8>,
    class: OperationClass,
    uncertain: Option<Arc<dyn UncertainReporter>>,
    page_callback: Option<Arc<dyn PageCallBack>>,
    sent: Arc<AtomicBool>,
    reported: Arc<std::sync::Mutex<ReportedSpend>>,
    answer: oneshot::Sender<Result<Received, CallFailure>>,
}

/// A caller abandoning a call that was timed out or dropped.
struct Cancel {
    id: RequestId,
    deadline: bool,
    ack: Option<oneshot::Sender<()>>,
}

/// Cancels a call on drop if it neither completed nor was explicitly
/// cancelled at its deadline.
struct CancelGuard {
    id: RequestId,
    cancels: mpsc::UnboundedSender<Cancel>,
    armed: bool,
}

impl CancelGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cancels.send(Cancel {
                id: self.id.clone(),
                deadline: false,
                ack: None,
            });
        }
    }
}

impl PluginInstance {
    /// Starts the instance of `plugin` for `scope` as a task of `owner`:
    /// it lives until `owner` is stopped, [`stop`](Self::stop) is called or
    /// every handle is dropped. Calls made before its `hello` is answered
    /// wait for it.
    #[must_use]
    pub fn start(owner: &TaskOwner, plugin: Plugin, scope: Scope) -> Self {
        Self::start_with(owner, plugin, scope, InstanceHooks::default())
    }

    /// Starts the instance of `plugin` for `scope` with `hooks` (audit log,
    /// quarantine notifier, resource monitor, proxied stream counter).
    #[must_use]
    pub fn start_with(
        owner: &TaskOwner,
        plugin: Plugin,
        scope: Scope,
        hooks: InstanceHooks,
    ) -> Self {
        let start_process = ProcessStart {
            launcher: plugin.launcher,
            launch: plugin.launch,
        };
        let config = Config {
            manifest: plugin.manifest,
            product: plugin.product,
            fetch: plugin.fetch,
        };
        let (handle, running, span) =
            Self::prepare(owner, config, scope, plugin.clock, plugin.spend, hooks);
        owner.spawn(
            running
                .run_lifecycle(TransportFactory::Process(Box::new(start_process)))
                .instrument(span),
        );
        handle
    }

    /// Starts a single-peer in-memory instance of `plugin` for `scope` as a
    /// task of `owner`, speaking the plugin protocol over duplex streams rather
    /// than a child process, so the host's rules can run on `Clock::paused`.
    #[must_use]
    pub fn start_in_memory(
        owner: &TaskOwner,
        plugin: InMemoryPlugin,
        scope: Scope,
    ) -> (Self, InMemoryPeer) {
        let (engine_write, from_engine) = tokio::io::duplex(READ_BYTES);
        let (to_engine, engine_read) = tokio::io::duplex(READ_BYTES);
        let config = Config {
            manifest: plugin.manifest,
            product: plugin.product,
            fetch: plugin.fetch,
        };
        let (handle, running, span) = Self::prepare(
            owner,
            config,
            scope,
            plugin.clock,
            plugin.spend,
            InstanceHooks::default(),
        );
        owner.spawn(
            running
                .run_lifecycle(TransportFactory::InMemorySingle(Some((
                    engine_write,
                    engine_read,
                ))))
                .instrument(span),
        );
        (
            handle,
            InMemoryPeer {
                from_engine,
                to_engine,
            },
        )
    }

    /// Starts a restartable in-memory instance of `plugin` for `scope` with
    /// `hooks`, yielding a fresh [`InMemoryPeer`] on the returned receiver for
    /// the initial start and every restart after backoff (ADR 0033, issue 837).
    #[must_use]
    pub fn start_in_memory_restarting(
        owner: &TaskOwner,
        plugin: InMemoryPlugin,
        scope: Scope,
        hooks: InstanceHooks,
    ) -> (Self, mpsc::UnboundedReceiver<InMemoryPeer>) {
        let (peers_tx, peers_rx) = mpsc::unbounded_channel();
        let config = Config {
            manifest: plugin.manifest,
            product: plugin.product,
            fetch: plugin.fetch,
        };
        let (handle, running, span) =
            Self::prepare(owner, config, scope, plugin.clock, plugin.spend, hooks);
        owner.spawn(
            running
                .run_lifecycle(TransportFactory::InMemoryRestarting(peers_tx))
                .instrument(span),
        );
        (handle, peers_rx)
    }

    /// Builds the shared handle, actor task and tracing span for both
    /// process and in-memory instances.
    fn prepare(
        owner: &TaskOwner,
        config: Config,
        scope: Scope,
        clock: Clock,
        spend: Option<PluginSpend>,
        hooks: InstanceHooks,
    ) -> (Self, Running, tracing::Span) {
        let (calls, mailbox) = mpsc::channel(MAILBOX);
        let (cancels, cancelled_calls) = mpsc::unbounded_channel();
        let (ending, ended) = watch::channel(None);
        let stop_reason = Arc::new(std::sync::Mutex::new(None));
        let declared_spend =
            DeclaredSpend::new(config.manifest.name.clone(), &config.manifest.spend, spend);
        let own = owner.child();
        let stopping = own.token().child_token();
        let span = tracing::info_span!(
            "plugin_instance",
            plugin_instance = %config.manifest.name,
            scope = %scope,
        );
        let running = Running {
            owner: own,
            stopping: stopping.clone(),
            stop_reason: Arc::clone(&stop_reason),
            config,
            scope,
            clock,
            hooks,
            crash_tracker: CrashTracker::new(),
            quarantine_notified: false,
            next_ping_id: -1,
            mailbox,
            cancels: cancelled_calls,
            ending,
        };
        let handle = Self {
            calls,
            cancels,
            next: Arc::new(AtomicI64::new(HELLO_ID + 1)),
            clock,
            spend: declared_spend,
            stopping,
            stop_reason,
            ended,
        };
        (handle, running, span)
    }

    /// Calls `method` with `params` and waits for the Plugin's answer, with
    /// no call-level deadline.
    ///
    /// # Errors
    ///
    /// [`CallFailure::Answered`] with the Plugin's error,
    /// [`CallFailure::Malformed`] when its answer cannot be read, and
    /// [`CallFailure::Unavailable`] with why the instance carried no call
    /// out.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the call was sent cancels it with
    /// `$/cancel`, releases its credit, and refuses any late result.
    pub async fn call(
        &self,
        method: &str,
        params: &impl Serialize,
    ) -> Result<Received, CallFailure> {
        self.call_under(method, params, CallOptions::default())
            .await
    }

    /// Calls `method` with `params` under `options`: holds the call to the
    /// manifest's concurrency credits in FIFO order, bounds its wait by its
    /// deadline and enclosing lease, abandons it with `$/cancel` if either
    /// passes first, refuses any late result, and reports an abandoned
    /// Effect-class call uncertain at once.
    ///
    /// # Errors
    ///
    /// [`CallFailure::Answered`] with the Plugin's error,
    /// [`CallFailure::Malformed`] when its answer cannot be read,
    /// [`CallFailure::DeadlineExceeded`] when the call's deadline or its
    /// enclosing lease passed first, and [`CallFailure::Unavailable`] when
    /// the lease already ended or the instance carried no call out.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the call was sent cancels it with
    /// `$/cancel`, releases its credit, refuses any late result, and
    /// reports an Effect-class call uncertain at once.
    pub async fn call_under(
        &self,
        method: &str,
        params: &impl Serialize,
        options: CallOptions,
    ) -> Result<Received, CallFailure> {
        if let Some(lease) = &options.lease
            && let Err(ended) = lease.check()
        {
            return Err(CallFailure::Unavailable(lease.withheld(ended, method)));
        }
        let call_spend = match &self.spend {
            Some(declared) => Some(declared.reserve().await?),
            None => None,
        };
        let id = RequestId::Number(self.next.fetch_add(1, Ordering::Relaxed));
        let line = wire::request_line(&id, method, params).map_err(|error| {
            CallFailure::Unavailable(internal(&format!(
                "the params of `{method}` encode: {error}"
            )))
        })?;
        let sent = Arc::new(AtomicBool::new(false));
        let reported = Arc::new(std::sync::Mutex::new(ReportedSpend::default()));
        let (answer, answered) = oneshot::channel();
        let call = Call {
            id: id.clone(),
            method: method.to_owned(),
            line,
            class: options.class,
            uncertain: options.uncertain,
            page_callback: options.page_callback,
            sent: Arc::clone(&sent),
            reported: Arc::clone(&reported),
            answer,
        };
        let mut guard = CancelGuard {
            id: id.clone(),
            cancels: self.cancels.clone(),
            armed: true,
        };
        let enclosing = options
            .enclosing
            .or_else(|| options.lease.as_deref().and_then(LeaseCheck::enclosing));
        let work = async {
            if self.calls.send(call).await.is_err() {
                return Err(self.end());
            }
            answered.await.unwrap_or_else(|_| Err(self.end()))
        };
        let timed = bounded(
            &self.clock,
            options.deadline.as_ref(),
            enclosing.as_ref(),
            work,
        )
        .await;
        guard.disarm();
        let outcome = match timed {
            Ok(result) => result,
            Err(deadline_error) => {
                let (ack_tx, ack_rx) = oneshot::channel();
                if self
                    .cancels
                    .send(Cancel {
                        id,
                        deadline: true,
                        ack: Some(ack_tx),
                    })
                    .is_ok()
                {
                    let _ = ack_rx.await;
                }
                Err(CallFailure::DeadlineExceeded(deadline_error))
            }
        };
        if let Some(spend) = call_spend {
            let was_sent = sent.load(Ordering::Acquire);
            let answered = matches!(
                outcome,
                Ok(_) | Err(CallFailure::Answered(_) | CallFailure::Malformed(_))
            );
            let reported = reported.lock().map(|slot| slot.clone()).unwrap_or_default();
            let settled = spend.finish(was_sent, answered, reported).await;
            if outcome.is_ok()
                && let Err(refusal) = settled
            {
                return Err(refusal);
            }
        }
        outcome
    }

    /// Calls `method` with `params` under `options`, retrying on retryable
    /// crashes (`plugin_crashed`) while `options.class` is `Read` or
    /// `IdempotentWrite` and the call's lease and deadline still hold (ADRs
    /// 0010, 0029 and 0033).
    ///
    /// An `Effect`-class call is never retried blindly, and a non-retryable
    /// failure (`plugin_limit_exceeded`, `plugin_quarantined`, etc.) is
    /// returned immediately.
    ///
    /// # Errors
    ///
    /// Same as [`Self::call_under`].
    pub async fn call_retrying(
        &self,
        method: &str,
        params: &impl Serialize,
        options: CallOptions,
    ) -> Result<Received, CallFailure> {
        if options.class == OperationClass::Effect {
            return self.call_under(method, params, options).await;
        }
        loop {
            match self.call_under(method, params, options.clone()).await {
                Err(CallFailure::Unavailable(ref error)) if error.retryable() => {
                    if self.ended().is_some() {
                        return Err(CallFailure::Unavailable(error.clone()));
                    }
                }
                other => return other,
            }
        }
    }

    /// Stops the instance and waits until its process is gone and its
    /// private temporary folder deleted.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future still stops the instance.
    pub async fn stop(&self) {
        self.stopping.cancel();
        let mut ended = self.ended.clone();
        // An instance whose task is gone has ended too.
        let _ = ended.wait_for(Option::is_some).await;
    }

    /// Stops the instance immediately with an explicit refusal `reason`
    /// (such as `plugin_revoked` when a revocation reaches a running
    /// Plugin) so in-flight and later calls report `reason`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future still stops the instance with `reason`.
    pub async fn stop_with(&self, reason: CallError) {
        if let Ok(mut slot) = self.stop_reason.lock()
            && slot.is_none()
        {
            *slot = Some(reason);
        }
        self.stop().await;
    }

    /// Why the instance ended, once it has.
    #[must_use]
    pub fn ended(&self) -> Option<CallError> {
        self.ended.borrow().clone()
    }

    /// The failure of a call the instance can no longer carry out.
    fn end(&self) -> CallFailure {
        CallFailure::Unavailable(self.ended().unwrap_or_else(cancelled))
    }
}

/// Awaits `work` bounded by `deadline` and `enclosing` (whichever passes
/// first), returning `Err(CallError)` on `deadline_exceeded`.
async fn bounded(
    clock: &Clock,
    deadline: Option<&Deadline>,
    enclosing: Option<&Enclosing>,
    work: impl Future<Output = Result<Received, CallFailure>>,
) -> Result<Result<Received, CallFailure>, CallError> {
    let timed = async {
        match deadline {
            Some(deadline) => deadline.within(clock, WaitedFor::PLUGIN_ANSWER, work).await,
            None => Ok(work.await),
        }
    };
    match enclosing {
        Some(enclosing) => enclosing.bound(clock, timed, || None).await,
        None => timed.await,
    }
}

/// The static configuration shared by process and in-memory instances.
struct Config {
    manifest: PluginManifest,
    product: Product,
    fetch: Arc<PluginFetch>,
}

/// What launches a confined child process for a process instance.
struct ProcessStart {
    launcher: PathBuf,
    launch: Launch,
}

/// What runs the Plugin on the other end of the protocol pipes.
enum Backend {
    Process(Box<PluginProcess>),
    InMemory,
}

impl Backend {
    /// Waits for a process backend to end; an in-memory backend ends when
    /// its stdout closes.
    async fn ended(&mut self) -> std::io::Result<Ended> {
        match self {
            Self::Process(process) => process.ended().await,
            Self::InMemory => std::future::pending().await,
        }
    }

    /// Whether this backend ends immediately when stdout reaches EOF.
    fn is_in_memory(&self) -> bool {
        matches!(self, Self::InMemory)
    }
}

/// How each generation of a [`PluginInstance`] obtains its transport pipes.
enum TransportFactory {
    Process(Box<ProcessStart>),
    InMemorySingle(Option<(DuplexStream, DuplexStream)>),
    InMemoryRestarting(mpsc::UnboundedSender<InMemoryPeer>),
}

impl TransportFactory {
    fn can_restart(&self) -> bool {
        !matches!(self, Self::InMemorySingle(_))
    }

    fn folder(&self) -> Option<PathBuf> {
        match self {
            Self::Process(start) => Some(start.launch.folder.clone()),
            Self::InMemorySingle(_) | Self::InMemoryRestarting(_) => None,
        }
    }
}

/// The instance's task.
struct Running {
    /// The instance's own owner, under the one it started on.
    owner: TaskOwner,
    /// Cancelled when its owner is stopped or [`PluginInstance::stop`] is
    /// called.
    stopping: CancellationToken,
    /// Explicit refusal reason set by [`PluginInstance::stop_with`], if any.
    stop_reason: Arc<std::sync::Mutex<Option<CallError>>>,
    config: Config,
    scope: Scope,
    clock: Clock,
    hooks: InstanceHooks,
    crash_tracker: CrashTracker,
    quarantine_notified: bool,
    next_ping_id: i64,
    mailbox: mpsc::Receiver<Call>,
    cancels: mpsc::UnboundedReceiver<Cancel>,
    ending: watch::Sender<Option<CallError>>,
}

/// The instance's state and the data beside it.
struct Instance {
    state: InstanceState,
    /// The maximum number of concurrent calls the Plugin may hold at once.
    max_concurrency: usize,
    /// Calls waiting for `hello` or for a free concurrency credit, in FIFO
    /// order.
    queued: VecDeque<Call>,
    /// Calls sent, by id, until answered or abandoned.
    waiting: BTreeMap<RequestId, Call>,
    /// Ids of calls abandoned in flight whose late results are refused
    /// (`late_result`), bounded by [`MAX_ABANDONED`].
    abandoned_set: BTreeSet<RequestId>,
    abandoned_order: VecDeque<RequestId>,
    /// Whether its `hello` awaits its answer.
    hello_awaited: bool,
    /// Unanswered `$/ping` request ids sent to the current process generation.
    pending_pings: BTreeSet<RequestId>,
    /// Consecutive ping intervals elapsed without any `$/ping` answer.
    missed_pings: u32,
    /// The Plugin's call backs that run.
    call_backs: CallBacks,
    /// Open `net.connect` streams carried as `net.stream.*` frames.
    streams: StreamRegistry,
}

/// The Plugin's call backs that run, each a task of the instance's owner.
struct CallBacks {
    /// What cancels those serving each call, by the call's id: cancelled
    /// once the call is answered or abandoned.
    serving: BTreeMap<RequestId, CancellationToken>,
    /// How many run.
    running: usize,
    /// Where each sends the line that answers it.
    answers: mpsc::Sender<Vec<u8>>,
}

impl CallBacks {
    /// Cancels the call backs that serve the call `id`, which is answered
    /// or abandoned.
    fn answered(&mut self, id: &RequestId) {
        if let Some(serving) = self.serving.remove(id) {
            serving.cancel();
        }
    }

    /// Cancels every active call back across all calls (on crash or restart).
    fn cancel_all(&mut self) {
        for (_, serving) in std::mem::take(&mut self.serving) {
            serving.cancel();
        }
        self.running = 0;
    }
}

impl Instance {
    /// A new instance in `Starting`, bounded to `max_concurrency` in-flight
    /// calls.
    fn new(
        max_concurrency: u32,
        answers: mpsc::Sender<Vec<u8>>,
        stream_events: mpsc::Sender<StreamEvent>,
    ) -> Self {
        Self {
            state: InstanceState::Starting,
            max_concurrency: usize::try_from(max_concurrency)
                .unwrap_or(usize::MAX)
                .max(1),
            queued: VecDeque::new(),
            waiting: BTreeMap::new(),
            abandoned_set: BTreeSet::new(),
            abandoned_order: VecDeque::new(),
            hello_awaited: true,
            pending_pings: BTreeSet::new(),
            missed_pings: 0,
            call_backs: CallBacks {
                serving: BTreeMap::new(),
                running: 0,
                answers,
            },
            streams: StreamRegistry::new(stream_events),
        }
    }

    /// Resets per-process-generation state when starting a replacement process
    /// while keeping `state` and `queued` calls intact.
    fn reset_generation(
        &mut self,
        answers: mpsc::Sender<Vec<u8>>,
        stream_events: mpsc::Sender<StreamEvent>,
    ) {
        self.abandoned_set.clear();
        self.abandoned_order.clear();
        self.hello_awaited = true;
        self.pending_pings.clear();
        self.missed_pings = 0;
        self.call_backs.cancel_all();
        self.call_backs.answers = answers;
        self.streams.close_all();
        self.streams = StreamRegistry::new(stream_events);
    }

    /// Whether an answer with `id` is awaited: its `hello`'s, a `$/ping`'s,
    /// or a call's.
    fn awaits(&self, id: &RequestId) -> bool {
        (self.hello_awaited && *id == RequestId::Number(HELLO_ID))
            || self.pending_pings.contains(id)
            || self.waiting.contains_key(id)
    }

    /// Records `id` as an abandoned in-flight call so its late result is
    /// refused.
    fn mark_abandoned(&mut self, id: RequestId) {
        if self.abandoned_set.insert(id.clone()) {
            self.abandoned_order.push_back(id);
            while self.abandoned_order.len() > MAX_ABANDONED {
                if let Some(oldest) = self.abandoned_order.pop_front() {
                    self.abandoned_set.remove(&oldest);
                }
            }
        }
    }

    /// Whether `id` is an abandoned call.
    fn is_abandoned(&self, id: &RequestId) -> bool {
        self.abandoned_set.contains(id)
    }

    /// Removes `id` from the abandoned set if present.
    fn take_abandoned(&mut self, id: &RequestId) -> bool {
        self.abandoned_set.remove(id)
    }

    /// Applies `event` to its state through its transition table, a
    /// refusal only a bug can cause logged at `error`.
    fn apply(&mut self, event: InstanceEvent) -> Result<InstanceEffect, InvalidInstanceTransition> {
        let (state, effect) =
            lifecycle::apply(self.state, event).map_err(InvalidInstanceTransition::logged)?;
        self.state = state;
        Ok(effect)
    }
}

/// Why a single process/peer generation exited.
enum Exit {
    /// Its owner was stopped, [`PluginInstance::stop`] called or every
    /// handle dropped: the event it was.
    Stopped(InstanceEvent),
    /// A fatal refusal (`hello` mismatch, `confinement_unavailable`, encoding
    /// error) that stops the instance without restarting.
    Failed(CallError),
    /// The process crashed, hit a resource limit, or violated an abandoned-call
    /// rule: handled under the crash/backoff/quarantine rules.
    Crashed {
        /// Whether this was a missed heartbeat (`PingMissed`) rather than a
        /// process exit or kill (`Crash`).
        ping_missed: bool,
        /// Failure error delivered to in-flight calls.
        reason: CallError,
    },
}

/// Active per-generation runtime handles dropped when a generation ends.
struct GenerationGuards {
    #[cfg(target_os = "linux")]
    _proxy: Option<crate::plugin_gate::PluginProxy>,
    proxied_streams: Option<OpenStreams>,
    monitor: Option<Arc<dyn ResourceMonitor>>,
}

impl Running {
    /// Handles a crash, resource-limit kill, or missed heartbeat under the
    /// crash/backoff/quarantine rules, returning `Some(final_reason)` when the
    /// instance ends or `None` to launch a replacement generation.
    async fn handle_crash(
        &mut self,
        instance: &mut Instance,
        can_restart: bool,
        ping_missed: bool,
        reason: CallError,
    ) -> Option<CallError> {
        let waiting = std::mem::take(&mut instance.waiting);
        for call in waiting.into_values() {
            self.report_if_effect(&call);
            let _ = call
                .answer
                .send(Err(CallFailure::Unavailable(reason.clone())));
        }
        if !can_restart {
            let _ = instance.apply(InstanceEvent::Crash(RecentCrashes::Fewer));
            return Some(reason);
        }
        let decision = self.crash_tracker.record(self.clock.now());
        let event = if ping_missed {
            InstanceEvent::PingMissed(decision.recent())
        } else {
            InstanceEvent::Crash(decision.recent())
        };
        match instance.apply(event) {
            Ok(InstanceEffect::Restart) => {
                let CrashDecision::Restart { wait } = decision else {
                    return Some(internal("restart effect without restart decision"));
                };
                if let Some(stop_exit) = self.wait_backoff(instance, wait).await {
                    return Some(match stop_exit {
                        Exit::Stopped(ev) => {
                            let _ = instance.apply(ev);
                            self.take_stop_reason()
                        }
                        Exit::Failed(err) | Exit::Crashed { reason: err, .. } => err,
                    });
                }
                None
            }
            Ok(InstanceEffect::Quarantine) => {
                let crashes = match decision {
                    CrashDecision::Quarantine { crashes } => crashes,
                    CrashDecision::Restart { .. } => self.crash_tracker.total_crashes(),
                };
                self.enter_quarantine(instance, crashes, &reason).await;
                let stop_event = self.serve_quarantined(instance).await;
                let _ = instance.apply(stop_event);
                Some(self.take_stop_reason())
            }
            Ok(InstanceEffect::Nothing) => None,
            Ok(other) => Some(internal(&format!("crash event led to {other:?}"))),
            Err(refused) => Some(internal(&refused.to_string())),
        }
    }

    fn take_stop_reason(&self) -> CallError {
        self.stop_reason
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .unwrap_or_else(cancelled)
    }

    fn exit_from_ended(&self, ended: std::io::Result<Ended>) -> Exit {
        match ended {
            Ok(Ended::ConfinementUnavailable) => Exit::Failed(code(
                &CONFINEMENT_UNAVAILABLE,
                "the host cannot confine the Plugin as its manifest declares",
            )),
            Ok(Ended::LimitExceeded(resource)) => Exit::Crashed {
                ping_missed: false,
                reason: limit_exceeded_error(
                    &self.config.manifest.name,
                    resource,
                    limits::limit_value(&self.config.manifest.limits, resource),
                ),
            },
            Ok(Ended::Exited(status)) => Exit::Crashed {
                ping_missed: false,
                reason: code(
                    &PLUGIN_CRASHED,
                    &format!("the Plugin exited with the status {status:?}"),
                ),
            },
            Err(error) => Exit::Crashed {
                ping_missed: false,
                reason: code(
                    &PLUGIN_CRASHED,
                    &format!("the Plugin's end cannot be told: {error}"),
                ),
            },
        }
    }

    /// Drives the full lifecycle of a Plugin instance across crashes, backoff
    /// waits, restarts, and quarantine until `run_ended`, `stop`, a fatal
    /// `hello`/confinement refusal, or all handles being dropped (ADR 0033).
    async fn run_lifecycle(mut self, mut factory: TransportFactory) {
        let (dummy_answers, _) = mpsc::channel(CALL_BACKS);
        let (dummy_streams, _) = mpsc::channel(STREAM_EVENTS);
        let mut instance = Instance::new(
            self.config.manifest.limits.concurrency,
            dummy_answers,
            dummy_streams,
        );
        let folder = factory.folder();
        let final_reason: CallError = loop {
            let (answers, call_backs_answered) = mpsc::channel(CALL_BACKS);
            let (stream_tx, stream_rx) = mpsc::channel(STREAM_EVENTS);
            instance.reset_generation(answers, stream_tx);

            let exit = match &mut factory {
                TransportFactory::Process(start) => {
                    self.run_process_generation(
                        &mut instance,
                        start,
                        call_backs_answered,
                        stream_rx,
                    )
                    .await
                }
                TransportFactory::InMemorySingle(pipes) => {
                    let Some((stdin, stdout)) = pipes.take() else {
                        break code(&PLUGIN_CRASHED, "the in-memory Plugin ended");
                    };
                    self.run_in_memory_generation(
                        &mut instance,
                        stdin,
                        stdout,
                        call_backs_answered,
                        stream_rx,
                    )
                    .await
                }
                TransportFactory::InMemoryRestarting(peers_tx) => {
                    let (engine_write, from_engine) = tokio::io::duplex(READ_BYTES);
                    let (to_engine, engine_read) = tokio::io::duplex(READ_BYTES);
                    if peers_tx
                        .send(InMemoryPeer {
                            from_engine,
                            to_engine,
                        })
                        .is_err()
                    {
                        break cancelled();
                    }
                    self.run_in_memory_generation(
                        &mut instance,
                        engine_write,
                        engine_read,
                        call_backs_answered,
                        stream_rx,
                    )
                    .await
                }
            };

            instance.streams.close_all();
            instance.call_backs.cancel_all();

            match exit {
                Exit::Stopped(event) => {
                    let _ = instance.apply(event);
                    break self.take_stop_reason();
                }
                Exit::Failed(reason) => break reason,
                Exit::Crashed {
                    ping_missed,
                    reason,
                } => {
                    if let Some(done) = self
                        .handle_crash(&mut instance, factory.can_restart(), ping_missed, reason)
                        .await
                    {
                        break done;
                    }
                }
            }
        };

        if let Some(folder) = folder {
            self.forget_folder(folder).await;
        }
        self.end(&final_reason, instance.queued, instance.waiting);
    }

    async fn run_in_memory_generation(
        &mut self,
        instance: &mut Instance,
        mut stdin: DuplexStream,
        stdout: DuplexStream,
        call_backs_answered: mpsc::Receiver<Vec<u8>>,
        stream_rx: mpsc::Receiver<StreamEvent>,
    ) -> Exit {
        let guards = GenerationGuards {
            #[cfg(target_os = "linux")]
            _proxy: None,
            proxied_streams: self.hooks.proxied_streams.clone(),
            monitor: self.hooks.resource_monitor.clone(),
        };
        let mut backend = Backend::InMemory;
        self.run_generation_pipes(
            instance,
            &mut backend,
            &guards,
            (&mut stdin, stdout, call_backs_answered, stream_rx),
        )
        .await
    }

    async fn run_process_generation(
        &mut self,
        instance: &mut Instance,
        start: &ProcessStart,
        call_backs_answered: mpsc::Receiver<Vec<u8>>,
        stream_rx: mpsc::Receiver<StreamEvent>,
    ) -> Exit {
        // Ensure the instance's private temporary folder exists before every start.
        let folder = start.launch.folder.clone();
        let _ = self
            .owner
            .blocking(move || {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "runs on the tasks module's blocking policy before launching"
                )]
                std::fs::create_dir_all(&folder)
            })
            .await;

        let mut launch = start.launch.clone();
        #[cfg(target_os = "linux")]
        let (proxy_guard, proxied_streams, started) = if self.config.manifest.network_level
            == goodfetch_protocol::plugin::NetworkLevel::ProxyTcp
            && launch.proxy_relay.is_none()
        {
            match crate::plugin_gate::PluginProxy::start_linux_relay(
                &self.owner,
                self.config.manifest.name.clone(),
                Arc::clone(&self.config.fetch),
                self.config.fetch.stream_accountant(),
            ) {
                Ok((proxy, control_fd)) => {
                    let creds = proxy.credentials();
                    let counter = proxy.open_streams_counter();
                    launch.proxy_relay = Some(goodfetch_confinement::launch::ProxyRelay {
                        control_fd: goodfetch_confinement::linux::relay::CONTROL_FD,
                        username: creds.username.clone(),
                        password: creds.password.clone(),
                    });
                    let started = PluginProcess::start_with_fd(
                        &start.launcher,
                        &launch,
                        control_fd,
                        goodfetch_confinement::linux::relay::CONTROL_FD,
                    );
                    (Some(proxy), Some(counter), started)
                }
                Err(error) => (None, None, Err(error)),
            }
        } else {
            (
                None,
                self.hooks.proxied_streams.clone(),
                PluginProcess::start(&start.launcher, &launch),
            )
        };
        #[cfg(not(target_os = "linux"))]
        let (proxied_streams, started) = (
            self.hooks.proxied_streams.clone(),
            PluginProcess::start(&start.launcher, &launch),
        );

        let (process, Pipes { mut stdin, stdout }) = match started {
            Ok(started) => started,
            Err(error) => {
                return Exit::Failed(internal(&format!(
                    "the Plugin's launcher did not start: {error}"
                )));
            }
        };

        let monitor: Option<Arc<dyn ResourceMonitor>> =
            if let Some(custom) = &self.hooks.resource_monitor {
                Some(Arc::clone(custom))
            } else {
                #[cfg(target_os = "linux")]
                {
                    process
                        .pid()
                        .and_then(limits::ProcGroupMonitor::for_pid)
                        .map(|m| Arc::new(m) as Arc<dyn ResourceMonitor>)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    None
                }
            };

        let guards = GenerationGuards {
            #[cfg(target_os = "linux")]
            _proxy: proxy_guard,
            proxied_streams,
            monitor,
        };
        let mut backend = Backend::Process(Box::new(process));
        let exit = self
            .run_generation_pipes(
                instance,
                &mut backend,
                &guards,
                (&mut stdin, stdout, call_backs_answered, stream_rx),
            )
            .await;
        drop(stdin);
        drop(guards);
        if let Backend::Process(process) = backend {
            process.kill_group().await;
        }
        exit
    }

    async fn run_generation_pipes<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
        &mut self,
        instance: &mut Instance,
        backend: &mut Backend,
        guards: &GenerationGuards,
        (stdin, stdout, call_backs_answered, stream_rx): (
            &mut W,
            R,
            mpsc::Receiver<Vec<u8>>,
            mpsc::Receiver<StreamEvent>,
        ),
    ) -> Exit {
        match self.hello_line() {
            Ok(line) => {
                write(&self.stopping, stdin, &line).await;
                self.serve(
                    instance,
                    backend,
                    guards,
                    (stdin, stdout, call_backs_answered, stream_rx),
                )
                .await
            }
            Err(reason) => Exit::Failed(reason),
        }
    }

    /// Waits for `wait` backoff before starting the replacement process while
    /// still accepting incoming calls into `instance.queued`, honouring
    /// cancellations, and stopping immediately on `run_ended` or `stop`.
    async fn wait_backoff(&mut self, instance: &mut Instance, wait: Duration) -> Option<Exit> {
        let sleep = self.clock.sleep(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                biased;
                () = self.stopping.cancelled() => {
                    let event = if self.owner.token().is_cancelled() {
                        self.scope.ended()
                    } else {
                        InstanceEvent::Stop
                    };
                    return Some(Exit::Stopped(event));
                }
                Some(cancel) = self.cancels.recv() => {
                    if let Some(pos) = instance.queued.iter().position(|call| call.id == cancel.id) {
                        instance.queued.remove(pos);
                    }
                    if let Some(ack) = cancel.ack {
                        let _ = ack.send(());
                    }
                }
                () = &mut sleep => return None,
                call = self.mailbox.recv() => {
                    let Some(call) = call else {
                        return Some(Exit::Stopped(InstanceEvent::Stop));
                    };
                    if call.answer.is_closed() {
                        continue;
                    }
                    match instance.apply(InstanceEvent::Call) {
                        Ok(InstanceEffect::Queue) => instance.queued.push_back(call),
                        Ok(effect) => answer_internal(call, effect),
                        Err(refused) => {
                            let _ = call.answer.send(Err(CallFailure::Unavailable(internal(
                                &refused.to_string(),
                            ))));
                        }
                    }
                }
            }
        }
    }

    /// Fails all queued calls with `plugin_quarantined`, appends the Audit log
    /// record, and fires the "a Plugin quarantined" Notification at most once.
    async fn enter_quarantine(
        &mut self,
        instance: &mut Instance,
        crashes: u32,
        reason: &CallError,
    ) {
        let quarantined = quarantined_error(&self.config.manifest.name);
        for call in instance.queued.drain(..) {
            let _ = call
                .answer
                .send(Err(CallFailure::Unavailable(quarantined.clone())));
        }
        let now = self.clock.timestamp();
        let occurred_at = self.clock.wall();
        if let Some(audit) = &self.hooks.audit {
            let origin = audit::Origin {
                actor: Actor::Engine,
                surface: None,
                layer: None,
                run_id: self.scope.run_id().cloned(),
            };
            let details = json!({
                "plugin": self.config.manifest.name,
                "crashes": crashes,
                "reason": reason.code(),
            });
            if let Err(error) = audit
                .record(&origin, &PLUGIN_QUARANTINED_AUDIT, details, now)
                .await
            {
                tracing::warn!(
                    plugin = %self.config.manifest.name,
                    %error,
                    "could not record `plugin_quarantined` in the Audit log"
                );
            }
        }
        if !self.quarantine_notified {
            self.quarantine_notified = true;
            if let Some(notifier) = &self.hooks.quarantine_notifier {
                notifier
                    .notify(QuarantineEvent {
                        plugin: self.config.manifest.name.clone(),
                        run_id: self.scope.run_id().cloned(),
                        product: self.config.product,
                        crashes,
                        reason: reason.code().to_owned(),
                        occurred_at,
                    })
                    .await;
            }
        }
    }

    /// Serves a quarantined instance until its Run or task stops, failing
    /// every incoming call immediately with `plugin_quarantined`.
    async fn serve_quarantined(&mut self, instance: &mut Instance) -> InstanceEvent {
        let quarantined = quarantined_error(&self.config.manifest.name);
        loop {
            tokio::select! {
                biased;
                () = self.stopping.cancelled() => {
                    return if self.owner.token().is_cancelled() {
                        self.scope.ended()
                    } else {
                        InstanceEvent::Stop
                    };
                }
                Some(cancel) = self.cancels.recv() => {
                    if let Some(ack) = cancel.ack {
                        let _ = ack.send(());
                    }
                }
                call = self.mailbox.recv() => {
                    let Some(call) = call else {
                        return InstanceEvent::Stop;
                    };
                    if call.answer.is_closed() {
                        continue;
                    }
                    let _ = instance.apply(InstanceEvent::Call);
                    let _ = call
                        .answer
                        .send(Err(CallFailure::Unavailable(quarantined.clone())));
                }
            }
        }
    }

    /// The line of its `hello`.
    fn hello_line(&self) -> Result<Vec<u8>, CallError> {
        let params = PluginHelloParams {
            protocol: PLUGIN_PROTOCOL,
            product: self.config.product,
            run_id: self.scope.run_id().cloned(),
            network_level: self.config.manifest.network_level,
        };
        wire::request_line(&RequestId::Number(HELLO_ID), HELLO, &params)
            .map_err(|error| internal(&format!("`hello` encodes: {error}")))
    }

    /// Serves one generation of the instance until it stops, crashes, misses
    /// its heartbeat, exceeds a resource limit, or fails `hello`.
    async fn serve<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
        &mut self,
        instance: &mut Instance,
        backend: &mut Backend,
        guards: &GenerationGuards,
        (stdin, mut stdout, mut call_backs_answered, mut stream_events): (
            &mut W,
            R,
            mpsc::Receiver<Vec<u8>>,
            mpsc::Receiver<StreamEvent>,
        ),
    ) -> Exit {
        let mut framer = Framer::new(MAX_MESSAGE_BYTES);
        let (mut buffer, mut parsed) = (vec![0; READ_BYTES], Vec::new());
        let mut reading = true;
        let mut next_ping: Option<tokio::time::Instant> = None;
        let mut next_sample = guards
            .monitor
            .is_some()
            .then(|| self.clock.now() + RESOURCE_POLL_INTERVAL);
        let clock = self.clock;
        loop {
            if instance.state == InstanceState::Ready && next_ping.is_none() {
                next_ping = Some(clock.now() + PING_INTERVAL);
            }
            let ping_at = next_ping.unwrap_or_else(|| clock.now() + PING_INTERVAL);
            let sample_at = next_sample.unwrap_or_else(|| clock.now() + RESOURCE_POLL_INTERVAL);
            tokio::select! {
                biased;
                () = self.stopping.cancelled() => {
                    let event = if self.owner.token().is_cancelled() {
                        self.scope.ended()
                    } else {
                        InstanceEvent::Stop
                    };
                    return Exit::Stopped(event);
                }
                // Cancellations run ahead of unread stdout frames so a
                // timed-out call is marked abandoned before any late result
                // is delivered.
                Some(cancel) = self.cancels.recv() => {
                    if let Some(exit) = self
                        .cancel_call(instance, stdin, cancel, guards.proxied_streams.as_ref())
                        .await
                    {
                        return exit;
                    }
                }
                // Before its end, so that what it said last is read.
                read = stdout.read(&mut buffer), if reading => {
                    match read {
                        // The Plugin closed its stdout: its end comes next.
                        Ok(0) | Err(_) => {
                            reading = false;
                            parsed.extend(framer.finish());
                        }
                        Ok(read) => framer.feed(&buffer[..read], &mut parsed),
                    }
                    for frame in parsed.drain(..) {
                        if let Some(exit) = self.received(instance, stdin, frame).await {
                            return exit;
                        }
                    }
                    if !reading && backend.is_in_memory() {
                        return Exit::Crashed {
                            ping_missed: false,
                            reason: code(
                                &PLUGIN_CRASHED,
                                "the Plugin closed its stdout",
                            ),
                        };
                    }
                }
                ended = backend.ended() => {
                    return self.exit_from_ended(ended);
                }
                () = clock.sleep_until(ping_at), if next_ping.is_some() => {
                    if let Some(exit) = self.on_ping_tick(instance, stdin).await {
                        return exit;
                    }
                    next_ping = Some(clock.now() + PING_INTERVAL);
                }
                () = clock.sleep_until(sample_at), if next_sample.is_some() => {
                    if let Some(exit) = self.check_resource_limits(guards) {
                        return exit;
                    }
                    next_sample = Some(clock.now() + RESOURCE_POLL_INTERVAL);
                }
                // The instance holds a sender, so this never ends.
                Some(line) = call_backs_answered.recv() => {
                    instance.call_backs.running = instance.call_backs.running.saturating_sub(1);
                    write(&self.stopping, stdin, &line).await;
                }
                Some(event) = stream_events.recv() => {
                    self.handle_stream_event(instance, stdin, event).await;
                }
                call = self.mailbox.recv() => {
                    let Some(call) = call else {
                        return Exit::Stopped(InstanceEvent::Stop);
                    };
                    self.called(instance, stdin, call).await;
                }
            }
        }
    }

    /// Handles one `PING_INTERVAL` tick on a ready instance: counts missed
    /// heartbeats and sends the next `$/ping`.
    async fn on_ping_tick<W: AsyncWrite + Unpin>(
        &mut self,
        instance: &mut Instance,
        stdin: &mut W,
    ) -> Option<Exit> {
        if !instance.pending_pings.is_empty() {
            instance.missed_pings = instance.missed_pings.saturating_add(1);
            if instance.missed_pings >= MISSED_PINGS {
                return Some(Exit::Crashed {
                    ping_missed: true,
                    reason: code(
                        &PLUGIN_CRASHED,
                        "the Plugin missed three consecutive `$/ping` heartbeats",
                    ),
                });
            }
        }
        let ping_id = RequestId::Number(self.next_ping_id);
        self.next_ping_id = self.next_ping_id.saturating_sub(1);
        if let Ok(line) = wire::request_line(&ping_id, PING, &PluginPingParams {}) {
            instance.pending_pings.insert(ping_id);
            write(&self.stopping, stdin, &line).await;
        }
        None
    }

    /// Samples the attached resource monitor and returns a `plugin_limit_exceeded`
    /// crash exit if any manifest limit is exceeded.
    fn check_resource_limits(&self, guards: &GenerationGuards) -> Option<Exit> {
        let monitor = guards.monitor.as_ref()?;
        let sample = monitor.sample()?;
        let (resource, limit) = limits::exceeded(&self.config.manifest.limits, &sample)?;
        Some(Exit::Crashed {
            ping_missed: false,
            reason: limit_exceeded_error(&self.config.manifest.name, resource, limit),
        })
    }

    /// Cancels a call whose deadline passed or whose caller dropped it:
    /// removes it from the credit queue if unsent, or abandons it in flight
    /// with `$/cancel`, cancels its call backs, reports an Effect-class
    /// call uncertain at once, and releases its credit to the next waiter — or
    /// restarts the instance under the crash rules when abandoned calls exceed
    /// concurrency or a proxied stream is open.
    async fn cancel_call<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        cancel: Cancel,
        proxied_streams: Option<&OpenStreams>,
    ) -> Option<Exit> {
        let mut restart_reason = None;
        if let Some(position) = instance.queued.iter().position(|call| call.id == cancel.id) {
            instance.queued.remove(position);
        } else if let Some(call) = instance.waiting.remove(&cancel.id) {
            instance.call_backs.answered(&cancel.id);
            instance.mark_abandoned(cancel.id.clone());
            let line = wire::notification_line(
                CANCEL,
                &PluginCancelParams {
                    call_id: cancel.id,
                    deadline: cancel.deadline,
                },
            );
            write(&self.stopping, stdin, &line).await;
            self.report_if_effect(&call);
            if proxied_streams.is_some_and(|streams| streams.count() > 0) {
                restart_reason =
                    Some("a call was abandoned while the Plugin held an open proxied stream");
            } else if instance.abandoned_set.len() > instance.max_concurrency {
                restart_reason =
                    Some("the Plugin holds more abandoned calls than its concurrency limit");
            } else {
                self.drain_queued(instance, stdin).await;
            }
        }
        if let Some(ack) = cancel.ack {
            let _ = ack.send(());
        }
        restart_reason.map(|msg| Exit::Crashed {
            ping_missed: false,
            reason: code(&PLUGIN_CRASHED, msg),
        })
    }

    /// Reports `call` uncertain at once when its class is `Effect`.
    fn report_if_effect(&self, call: &Call) {
        if call.class != OperationClass::Effect {
            return;
        }
        let uncertain = UncertainCall {
            plugin: self.config.manifest.name.clone(),
            call_id: call.id.clone(),
            method: call.method.clone(),
            reason: UncertainReason::Interrupted,
        };
        tracing::warn!(
            plugin = %uncertain.plugin,
            call_id = ?uncertain.call_id,
            method = %uncertain.method,
            reason = uncertain.reason.as_str(),
            "an abandoned Effect-class Plugin call is uncertain"
        );
        if let Some(reporter) = &call.uncertain {
            reporter.report(uncertain);
        }
    }

    /// Sends queued calls in FIFO order while the instance is ready and
    /// holds fewer than `max_concurrency` in-flight calls, skipping any
    /// whose caller has already gone away.
    async fn drain_queued<W: AsyncWrite + Unpin>(&self, instance: &mut Instance, stdin: &mut W) {
        while instance.state == InstanceState::Ready
            && instance.waiting.len() < instance.max_concurrency
        {
            let Some(call) = instance.queued.pop_front() else {
                break;
            };
            if call.answer.is_closed() {
                continue;
            }
            send(&self.stopping, instance, stdin, call).await;
        }
    }

    /// The Plugin's call back `call`, with the id `id`: `usage.report` records
    /// raw spend units synchronously, and `net.fetch` runs as a task of the
    /// instance's owner, whose answer comes back through [`CallBacks::answers`].
    async fn called_back<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        id: RequestId,
        call: &framing::Call,
    ) {
        if call.method() == USAGE_REPORT {
            let line = match self.record_usage_report(instance, call) {
                Ok(()) => match wire::success_line(&id, &UsageReportResult {}) {
                    Ok(line) => line,
                    Err(error) => wire::failure_line(
                        Some(id),
                        internal(&format!("a `{USAGE_REPORT}` result encodes: {error}"))
                            .to_error_object(),
                    ),
                },
                Err(error) => wire::failure_line(Some(id), error.to_error_object()),
            };
            return write(&self.stopping, stdin, &line).await;
        }
        if is_page_call_back(call.method()) {
            return self.serve_page_call_back(instance, stdin, id, call).await;
        }
        if call.method() == NET_CONNECT {
            return self.serve_net_connect(instance, stdin, id, call).await;
        }
        if self.config.manifest.kinds.contains(&PluginKindName::HOOK)
            && is_forbidden_hook_call_back(call.method())
        {
            let error = code(
                &PLUGIN_CALL_BACK_REFUSED,
                &format!(
                    "a Hook Plugin may only call `net.fetch` and its stage's `page.*` operations, never `{}`",
                    call.method()
                ),
            );
            return write(&self.stopping, stdin, &call_back_line(id, Some(Err(error)))).await;
        }
        let admitted = served(call).and_then(|params| {
            admit_call_back(
                &instance.waiting,
                instance.call_backs.running,
                &params.call_id,
            )
            .map(|()| params)
        });
        let params = match admitted {
            Ok(params) => params,
            Err(error) => {
                return write(&self.stopping, stdin, &call_back_line(id, Some(Err(error)))).await;
            }
        };
        let call_backs = &mut instance.call_backs;
        let stopped = call_backs
            .serving
            .entry(params.call_id.clone())
            .or_insert_with(|| self.stopping.child_token())
            .child_token();
        call_backs.running += 1;
        let answers = call_backs.answers.clone();
        let fetch = Arc::clone(&self.config.fetch);
        self.owner.spawn(async move {
            let outcome = stopped.run_until_cancelled(fetch.fetch(params)).await;
            // An instance that ended reads no answer.
            let _ = answers.send(call_back_line(id, outcome)).await;
        });
    }

    /// Serves a `net.connect` call back (ADR 0033, issue 833): refuses Hook
    /// and Extractor Plugins, admits the call under its active `call_id`, and
    /// opens the validated TCP stream on a spawned task bound to the call's
    /// lease token.
    async fn serve_net_connect<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        id: RequestId,
        call: &framing::Call,
    ) {
        if self
            .config
            .manifest
            .kinds
            .iter()
            .any(|k| *k == PluginKindName::HOOK || *k == PluginKindName::EXTRACT)
        {
            let error = code(
                &PLUGIN_CALL_BACK_REFUSED,
                "Hooks and Extractors may declare only `none` and never get `net.connect`",
            );
            return write(&self.stopping, stdin, &call_back_line(id, Some(Err(error)))).await;
        }
        let parsed = serde_json::from_str::<NetConnectParams>(call.params().unwrap_or("{}"))
            .map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("the params of `{NET_CONNECT}` cannot be read: {error}"),
                )
            })
            .and_then(|params| {
                admit_call_back(
                    &instance.waiting,
                    instance.call_backs.running,
                    &params.call_id,
                )
                .map(|()| params)
            });
        let params = match parsed {
            Ok(params) => params,
            Err(error) => {
                return write(&self.stopping, stdin, &call_back_line(id, Some(Err(error)))).await;
            }
        };
        let call_backs = &mut instance.call_backs;
        let stopped = call_backs
            .serving
            .entry(params.call_id.clone())
            .or_insert_with(|| self.stopping.child_token())
            .child_token();
        call_backs.running += 1;
        let events = instance.streams.sender();
        let fetch = Arc::clone(&self.config.fetch);
        self.owner.spawn(async move {
            let outcome = stopped
                .run_until_cancelled(fetch.connect(&params.host, params.port, None))
                .await;
            let _ = events
                .send(StreamEvent::Connected {
                    reply_id: id,
                    call_id: params.call_id,
                    host: params.host,
                    port: params.port,
                    outcome,
                })
                .await;
        });
    }

    /// Processes a completed `net.connect` or an outbound `net.stream.*`
    /// notification frame.
    async fn handle_stream_event<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        event: StreamEvent,
    ) {
        match event {
            StreamEvent::Connected {
                reply_id,
                call_id,
                host,
                port,
                outcome,
            } => {
                instance.call_backs.running = instance.call_backs.running.saturating_sub(1);
                let line = match outcome {
                    Some(Ok(tcp)) => {
                        if let Some(lease_token) = instance.call_backs.serving.get(&call_id)
                            && instance.waiting.contains_key(&call_id)
                        {
                            let stream_id = instance.streams.open(
                                &self.owner,
                                self.config.manifest.name.clone(),
                                &host,
                                port,
                                tcp,
                                lease_token.clone(),
                                self.config.fetch.clock(),
                                self.config.fetch.stream_accountant(),
                            );
                            let result = NetConnectResult {
                                stream_id,
                                credits: INITIAL_CREDITS,
                            };
                            match wire::success_line(&reply_id, &result) {
                                Ok(line) => line,
                                Err(error) => wire::failure_line(
                                    Some(reply_id),
                                    internal(&format!("a `{NET_CONNECT}` result encodes: {error}"))
                                        .to_error_object(),
                                ),
                            }
                        } else {
                            drop(tcp);
                            let error = code(
                                &PLUGIN_CALL_BACK_REFUSED,
                                "the call the Plugin's `net.connect` served was answered, or the instance stopped",
                            );
                            wire::failure_line(Some(reply_id), error.to_error_object())
                        }
                    }
                    Some(Err(error)) => wire::failure_line(Some(reply_id), error.to_error_object()),
                    None => {
                        let error = code(
                            &PLUGIN_CALL_BACK_REFUSED,
                            "the call the Plugin's `net.connect` served was answered, or the instance stopped",
                        );
                        wire::failure_line(Some(reply_id), error.to_error_object())
                    }
                };
                write(&self.stopping, stdin, &line).await;
            }
            StreamEvent::Frame(frame) => write(&self.stopping, stdin, &frame).await,
            StreamEvent::CreditsReturned {
                stream_id,
                bytes,
                frame,
            } => {
                instance.streams.credit_returned(&stream_id, bytes);
                write(&self.stopping, stdin, &frame).await;
            }
            StreamEvent::Closed { stream_id, frame } => {
                instance.streams.remove(&stream_id);
                write(&self.stopping, stdin, &frame).await;
            }
        }
    }

    /// Serves a `page.*` or `locator.*` call back from an in-flight Hook call.
    async fn serve_page_call_back<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        id: RequestId,
        call: &framing::Call,
    ) {
        let method = call.method().to_owned();
        let parsed = parse_page_call_back(call).and_then(|(call_id, params)| {
            admit_call_back(&instance.waiting, instance.call_backs.running, &call_id)?;
            let callback = instance
                .waiting
                .get(&call_id)
                .and_then(|active| active.page_callback.clone())
                .ok_or_else(|| {
                    code(
                        &PLUGIN_CALL_BACK_REFUSED,
                        &format!("`{method}` is not served outside a page-stage Hook call"),
                    )
                })?;
            Ok((call_id, params, callback))
        });
        let (call_id, params, callback) = match parsed {
            Ok(admitted) => admitted,
            Err(error) => {
                let line = wire::failure_line(Some(id), error.to_error_object());
                return write(&self.stopping, stdin, &line).await;
            }
        };
        let call_backs = &mut instance.call_backs;
        let stopped = call_backs
            .serving
            .entry(call_id)
            .or_insert_with(|| self.stopping.child_token())
            .child_token();
        call_backs.running += 1;
        let answers = call_backs.answers.clone();
        self.owner.spawn(async move {
            let outcome = stopped
                .run_until_cancelled(callback.call(&method, params))
                .await;
            let _ = answers
                .send(page_call_back_line(id, &method, outcome))
                .await;
        });
    }

    /// Validates and records a Plugin's `usage.report` call back against its
    /// active in-flight call.
    fn record_usage_report(
        &self,
        instance: &Instance,
        call: &framing::Call,
    ) -> Result<(), CallError> {
        let params: UsageReportParams = serde_json::from_str(call.params().unwrap_or("null"))
            .map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("the params of `{USAGE_REPORT}` cannot be read: {error}"),
                )
            })?;
        let Some(active) = instance.waiting.get(&params.call_id) else {
            return Err(code(
                &PLUGIN_CALL_BACK_REFUSED,
                &format!(
                    "`{USAGE_REPORT}` named call `{:?}`, which this instance does not await",
                    params.call_id
                ),
            ));
        };
        if !self.config.manifest.spend.contains_key(&params.unit) {
            return Err(code(
                &PLUGIN_CALL_BACK_REFUSED,
                &format!(
                    "`{USAGE_REPORT}` named unit `{}`, which `{}`'s manifest does not declare",
                    params.unit, self.config.manifest.name
                ),
            ));
        }
        if let Ok(mut reported) = active.reported.lock() {
            reported.add(params.unit, params.quantity);
        }
        Ok(())
    }

    /// A call its caller made.
    async fn called<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        call: Call,
    ) {
        if call.answer.is_closed() {
            return;
        }
        match instance.apply(InstanceEvent::Call) {
            Ok(InstanceEffect::Queue) => instance.queued.push_back(call),
            Ok(InstanceEffect::Send) => {
                if instance.waiting.len() < instance.max_concurrency && instance.queued.is_empty() {
                    send(&self.stopping, instance, stdin, call).await;
                } else {
                    instance.queued.push_back(call);
                }
            }
            Ok(effect) => answer_internal(call, effect),
            Err(refused) => {
                let reason = match refused.refusal {
                    Refusal::Program(_) => code(
                        &PLUGIN_QUARANTINED,
                        "the Plugin is quarantined for the rest of its run",
                    ),
                    Refusal::Internal => internal(&refused.to_string()),
                };
                let _ = call.answer.send(Err(CallFailure::Unavailable(reason)));
            }
        }
    }

    /// Handles a notification sent by the Plugin (`usage.report` or
    /// `net.stream.*`).
    async fn received_notification(&self, instance: &mut Instance, call: &crate::framing::Call) {
        let params = call.params().unwrap_or("{}");
        match call.method() {
            USAGE_REPORT => {
                if let Err(error) = self.record_usage_report(instance, call) {
                    stray(&format!("refused `{USAGE_REPORT}` notification: {error}"));
                }
            }
            NET_STREAM_DATA => match serde_json::from_str::<NetStreamDataParams>(params) {
                Ok(parsed) => instance.streams.on_data(parsed).await,
                Err(error) => stray(&format!("invalid `{NET_STREAM_DATA}`: {error}")),
            },
            NET_STREAM_CREDITS => match serde_json::from_str::<NetStreamCreditsParams>(params) {
                Ok(parsed) => instance.streams.on_credits(&parsed),
                Err(error) => stray(&format!("invalid `{NET_STREAM_CREDITS}`: {error}")),
            },
            NET_STREAM_CLOSE => match serde_json::from_str::<NetStreamCloseParams>(params) {
                Ok(parsed) => instance.streams.on_close(&parsed),
                Err(error) => stray(&format!("invalid `{NET_STREAM_CLOSE}`: {error}")),
            },
            method => {
                stray(&format!(
                    "the Plugin sent the notification `{method}`, which the host does not serve"
                ));
            }
        }
    }

    /// A frame the Plugin sent: the exit it leads to, if any.
    async fn received<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        frame: Frame,
    ) -> Option<Exit> {
        let line = match frame {
            Frame::Line(line) => line,
            Frame::TooLarge => {
                stray(&format!(
                    "a message is longer than {MAX_MESSAGE_BYTES} bytes"
                ));
                return None;
            }
        };
        let (id, answer) = match decode(line) {
            Ok(Incoming::Reply { id, answer }) => (id, answer.map_err(CallFailure::Answered)),
            Ok(Incoming::Request { id, call }) => {
                self.called_back(instance, stdin, id, &call).await;
                return None;
            }
            Ok(Incoming::Notification(call)) => {
                self.received_notification(instance, &call).await;
                return None;
            }
            Err(error) => match error.id {
                Some(id) if instance.awaits(&id) => (
                    id,
                    Err(CallFailure::Malformed(Malformed {
                        detail: error.detail,
                    })),
                ),
                Some(id) if instance.is_abandoned(&id) => {
                    instance.take_abandoned(&id);
                    let _ = instance.apply(InstanceEvent::LateResult);
                    late_result(&id);
                    return None;
                }
                _ => {
                    stray(&error.detail);
                    return None;
                }
            },
        };
        if id == RequestId::Number(HELLO_ID) && instance.hello_awaited {
            instance.hello_awaited = false;
            return self.hello_answered(instance, stdin, answer).await;
        }
        if instance.pending_pings.remove(&id) {
            if answer.is_ok() {
                instance.pending_pings.clear();
                instance.missed_pings = 0;
            }
            return None;
        }
        let Some(call) = instance.waiting.remove(&id) else {
            if instance.take_abandoned(&id) {
                let _ = instance.apply(InstanceEvent::LateResult);
                late_result(&id);
            } else {
                stray(&format!("the Plugin answered {id:?}, which no call awaits"));
            }
            return None;
        };
        instance.call_backs.answered(&id);
        match instance.apply(InstanceEvent::Result) {
            Ok(InstanceEffect::Deliver) => {
                // A caller that went away leaves the answer to nobody.
                let _ = call.answer.send(answer);
            }
            Ok(effect) => answer_internal(call, effect),
            Err(refused) => {
                let _ = call.answer.send(Err(CallFailure::Unavailable(internal(
                    &refused.to_string(),
                ))));
            }
        }
        self.drain_queued(instance, stdin).await;
        None
    }

    /// The Plugin's answer to `hello`: ready, or stopped when it differs
    /// from the approved manifest.
    async fn hello_answered<W: AsyncWrite + Unpin>(
        &self,
        instance: &mut Instance,
        stdin: &mut W,
        answer: Result<Received, CallFailure>,
    ) -> Option<Exit> {
        let checked = answer
            .map_err(|failure| failure.to_string())
            .and_then(|received| {
                received
                    .read::<PluginHelloResult>()
                    .map_err(|error| format!("the answer to `hello` cannot be read: {error}"))
            })
            .map(|result| check_hello(PLUGIN_PROTOCOL, &self.config.manifest, &result));
        let (event, refusal) = match checked {
            Ok(Ok(())) => (InstanceEvent::HelloOk, None),
            Ok(Err(refusal)) => (InstanceEvent::HelloRefused, Some(refused(&refusal))),
            Err(detail) => (
                InstanceEvent::HelloRefused,
                Some(code(
                    &PLUGIN_MANIFEST_MISMATCH,
                    &format!("the Plugin's `hello` is not its approved manifest's: {detail}"),
                )),
            ),
        };
        match instance.apply(event) {
            Ok(InstanceEffect::Open) => {
                self.drain_queued(instance, stdin).await;
                None
            }
            Ok(InstanceEffect::Refuse) => {
                Some(Exit::Failed(refusal.unwrap_or_else(|| {
                    internal("a `hello` refused with no reason")
                })))
            }
            Ok(effect) => Some(Exit::Failed(internal(&format!(
                "{event:?} led to {effect:?}"
            )))),
            Err(refused) => Some(Exit::Failed(internal(&refused.to_string()))),
        }
    }

    /// Deletes the instance's folder when its process never started.
    async fn forget_folder(&self, folder: PathBuf) {
        let deleted = self
            .owner
            .blocking(move || {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "it runs on the tasks module's blocking policy"
                )]
                std::fs::remove_dir_all(folder)
            })
            .await;
        if let Err(error) = deleted
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "the instance's private temporary folder was not deleted");
        }
    }

    /// Ends the instance with `reason`: every call it holds, and each one
    /// made later, fails with it, and every in-flight Effect-class call is
    /// reported uncertain at once.
    fn end(
        mut self,
        reason: &CallError,
        queued: VecDeque<Call>,
        waiting: BTreeMap<RequestId, Call>,
    ) {
        self.ending.send_replace(Some(reason.clone()));
        self.mailbox.close();
        for call in waiting.values() {
            self.report_if_effect(call);
        }
        let mut held: Vec<Call> = queued.into_iter().chain(waiting.into_values()).collect();
        while let Ok(call) = self.mailbox.try_recv() {
            held.push(call);
        }
        for call in held {
            let _ = call
                .answer
                .send(Err(CallFailure::Unavailable(reason.clone())));
        }
    }
}

/// Sends `call` to the Plugin and awaits its answer.
async fn send<W: AsyncWrite + Unpin>(
    stopping: &CancellationToken,
    instance: &mut Instance,
    stdin: &mut W,
    call: Call,
) {
    call.sent.store(true, Ordering::Release);
    write(stopping, stdin, &call.line).await;
    instance.waiting.insert(call.id.clone(), call);
}

/// Writes `line` to the Plugin, unless the instance is stopping. A write
/// the Plugin refuses fails its calls once its end shows.
async fn write<W: AsyncWrite + Unpin>(stopping: &CancellationToken, stdin: &mut W, line: &[u8]) {
    let written = async {
        stdin.write_all(line).await?;
        stdin.flush().await
    };
    if let Some(Err(error)) = stopping.run_until_cancelled(written).await {
        tracing::debug!(%error, "the Plugin refused a message");
    }
}

/// Logs and refuses an answer that arrived for a call already abandoned
/// (`late_result`, ADR 0003).
fn late_result(id: &RequestId) {
    tracing::warn!(
        code = "late_result",
        call_id = ?id,
        "refused a late result for an abandoned Plugin call"
    );
}

/// Whether `method` is a `page.*` or `locator.*` call back that a page-stage
/// Hook may invoke on its active page.
fn is_page_call_back(method: &str) -> bool {
    method.starts_with("page.") || method.starts_with("locator.")
}

/// Whether `method` is a capability explicitly forbidden to a Hook Plugin
/// (ADRs 0033, 0088 and 0104; issue 954): `net.connect`, the Run index,
/// settings, Operator policy, accounts, or direct Session management.
fn is_forbidden_hook_call_back(method: &str) -> bool {
    method == "net.connect"
        || method.starts_with("run.")
        || method.starts_with("config.")
        || method.starts_with("settings.")
        || method.starts_with("policy.")
        || method.starts_with("accounts.")
        || method.starts_with("session.")
}

/// Parses a `page.*` / `locator.*` call back's JSON params, extracting its
/// `call_id` and forwarding the remaining params object.
fn parse_page_call_back(call: &framing::Call) -> Result<(RequestId, serde_json::Value), CallError> {
    let mut value: serde_json::Value = serde_json::from_str(call.params().unwrap_or("null"))
        .map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("the params of `{}` cannot be read: {error}", call.method()),
            )
        })?;
    let Some(fields) = value.as_object_mut() else {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("the params of `{}` must be a JSON object", call.method()),
        ));
    };
    let raw_id = fields.remove("call_id").ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{}` must include `call_id`", call.method()),
        )
    })?;
    let call_id: RequestId = serde_json::from_value(raw_id).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{}` has an invalid `call_id`: {error}", call.method()),
        )
    })?;
    Ok((call_id, value))
}

/// Encodes the reply line for a Hook's `page.*` / `locator.*` call back.
fn page_call_back_line(
    id: RequestId,
    method: &str,
    outcome: Option<Result<serde_json::Value, CallError>>,
) -> Vec<u8> {
    let failure = match outcome {
        Some(Ok(result)) => match wire::success_line(&id, &result) {
            Ok(line) => return line,
            Err(error) => internal(&format!("a `{method}` result encodes: {error}")),
        },
        Some(Err(error)) => error,
        None => code(
            &CALL_CANCELLED,
            &format!(
                "the call the Plugin's `{method}` served was answered, or the instance stopped"
            ),
        ),
    };
    wire::failure_line(Some(id), failure.to_error_object())
}

/// The params of the Plugin's call back `call`, which the engine serves.
///
/// # Errors
///
/// `method_not_found` for a method the engine does not serve, and
/// `invalid_params` for params that are not the method's.
fn served(call: &framing::Call) -> Result<NetFetchParams, CallError> {
    if call.method() != NET_FETCH {
        return Err(CallError::protocol(
            ProtocolFailure::MethodNotFound,
            format!(
                "the engine serves a Plugin `{NET_FETCH}`, not `{}`",
                call.method()
            ),
        ));
    }
    serde_json::from_str(call.params().unwrap_or("null")).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("the params of `{NET_FETCH}` cannot be read: {error}"),
        )
    })
}

/// The line that answers the Plugin's call back `id` with `outcome`: its
/// result, its failure, or none when it was cancelled before it ended.
fn call_back_line(id: RequestId, outcome: Option<Result<NetFetchResult, CallError>>) -> Vec<u8> {
    let failure = match outcome {
        Some(Ok(result)) => match wire::success_line(&id, &result) {
            Ok(line) => return line,
            Err(error) => internal(&format!("a `net.fetch` result encodes: {error}")),
        },
        Some(Err(error)) => error,
        None => code(
            &CALL_CANCELLED,
            "the call the Plugin's `net.fetch` served was answered, or the instance stopped",
        ),
    };
    wire::failure_line(Some(id), failure.to_error_object())
}

/// Logs one of the Plugin's stray messages, handled as a malformed answer.
fn stray(detail: &str) {
    tracing::warn!(
        detail,
        "the Plugin sent a message that answers nothing the instance awaits, handled as a malformed answer"
    );
}

/// Fails `call` when its event led to an effect the table never gives it.
fn answer_internal(call: Call, effect: InstanceEffect) {
    let reason = internal(&format!("a call led to {effect:?}"));
    let _ = call.answer.send(Err(CallFailure::Unavailable(reason)));
}

/// The Error code of a refused `hello`.
fn refused(refusal: &HelloRefusal) -> CallError {
    let definition = if refusal.code() == ErrorCode::PLUGIN_PROTOCOL_UNSUPPORTED {
        &PLUGIN_PROTOCOL_UNSUPPORTED
    } else {
        &PLUGIN_MANIFEST_MISMATCH
    };
    code(definition, &refusal.to_string())
}

/// A failure with `definition`'s code and `message`.
fn code(definition: &'static ErrorCodeDefinition, message: &str) -> CallError {
    CallError::new(definition, message, &())
}

/// The `plugin_quarantined` refusal for `plugin`.
fn quarantined_error(plugin: &str) -> CallError {
    code(
        &PLUGIN_QUARANTINED,
        &format!("the Plugin `{plugin}` is quarantined for the rest of its run"),
    )
}

/// The non-retryable `plugin_limit_exceeded` error for `plugin` exceeding
/// `resource` at `limit`.
fn limit_exceeded_error(plugin: &str, resource: PluginResourceLimit, limit: u64) -> CallError {
    CallError::new(
        &PLUGIN_LIMIT_EXCEEDED,
        format!(
            "the Plugin `{plugin}` exceeded its `{}` resource limit ({limit}) and was killed",
            resource.as_str()
        ),
        &PluginLimitExceededDetails {
            plugin: plugin.to_owned(),
            resource,
            limit,
        },
    )
}

/// A failure only a bug in Goodfetch can cause.
fn internal(message: &str) -> CallError {
    code(&INTERNAL, message)
}

/// The failure of a call the instance holds when it stops.
fn cancelled() -> CallError {
    code(
        &CALL_CANCELLED,
        "the Plugin's instance stopped with its run or its task",
    )
}

#[cfg(test)]
mod tests;
