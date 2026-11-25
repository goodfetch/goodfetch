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
//! A message from the Plugin that answers nothing the instance awaits (a
//! second answer to `hello`, an answer to no call, a call of its own, which
//! the host does not serve yet) is decoded before the transition function
//! and handled as a malformed answer, logged at `warn` with
//! `plugin_instance`: never an event.
//!
//! Each call is encoded once, by its caller, into its line. Each answer is
//! decoded once by `rpc`'s framing, and its caller reads the result from
//! where it is in that line (ADR 0140's E1).
//!
//! Crashes, restarts, quarantine and the heartbeat come with #837. Until
//! then, a process that ends ends its instance: its calls fail with
//! `plugin_crashed`, or with `confinement_unavailable` when its launcher
//! found that the host cannot confine it.

#![expect(
    clippy::disallowed_types,
    reason = "the next call's id is taken in one step, never across an `.await` or under a lock, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use goodfetch_confinement::launch::Launch;
use goodfetch_protocol::catalog::{ErrorCodeDefinition, Product};
use goodfetch_protocol::connection::{CALL_CANCELLED, INTERNAL};
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::jsonrpc::{ErrorObject, RequestId};
use goodfetch_protocol::plugin::{
    CONFINEMENT_UNAVAILABLE, HELLO, HelloRefusal, PLUGIN_CRASHED, PLUGIN_MANIFEST_MISMATCH,
    PLUGIN_PROTOCOL, PLUGIN_PROTOCOL_UNSUPPORTED, PLUGIN_QUARANTINED, PluginHelloParams,
    PluginHelloResult, PluginManifest, check_hello,
};
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::lifecycle::{
    self, InstanceEffect, InstanceEvent, InstanceState, InvalidInstanceTransition, RecentCrashes,
};
use super::process::{Ended, Pipes, PluginProcess};
use crate::call_error::CallError;
use crate::framing::{Frame, Framer, Incoming, MAX_MESSAGE_BYTES, decode};
use crate::tasks::TaskOwner;
use crate::transition::Refusal;
use crate::wire::{self, Received};

/// The id of the engine's `hello`: each call's id is one of the next.
const HELLO_ID: i64 = 0;

/// How many calls its mailbox holds before a caller waits. The manifest's
/// concurrency limit and its credits bound what the Plugin holds (story
/// 7-32); this only bounds what waits to be sent.
const MAILBOX: usize = 64;

/// How many bytes it reads from the Plugin at once.
const READ_BYTES: usize = 64 * 1024;

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
    /// The instance did not carry the call out: refused at `hello`,
    /// crashed, quarantined, or stopped with its Run or its task.
    #[error(transparent)]
    Unavailable(CallError),
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
    next: Arc<AtomicI64>,
    stopping: CancellationToken,
    ended: watch::Receiver<Option<CallError>>,
}

/// A call on its way to the Plugin: its line, encoded once, and where its
/// answer goes.
struct Call {
    id: RequestId,
    line: Vec<u8>,
    answer: oneshot::Sender<Result<Received, CallFailure>>,
}

impl PluginInstance {
    /// Starts the instance of `plugin` for `scope` as a task of `owner`:
    /// it lives until `owner` is stopped, [`stop`](Self::stop) is called or
    /// every handle is dropped. Calls made before its `hello` is answered
    /// wait for it.
    #[must_use]
    pub fn start(owner: &TaskOwner, plugin: Plugin, scope: Scope) -> Self {
        let (calls, mailbox) = mpsc::channel(MAILBOX);
        let (ending, ended) = watch::channel(None);
        let own = owner.child();
        let stopping = own.token().child_token();
        let span = tracing::info_span!(
            "plugin_instance",
            plugin_instance = %plugin.manifest.name,
            scope = %scope,
        );
        let running = Running {
            owner: own,
            stopping: stopping.clone(),
            plugin,
            scope,
            mailbox,
            ending,
        };
        owner.spawn(running.run().instrument(span));
        Self {
            calls,
            next: Arc::new(AtomicI64::new(HELLO_ID + 1)),
            stopping,
            ended,
        }
    }

    /// Calls `method` with `params` and waits for the Plugin's answer.
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
    /// Dropping the future once the call was sent leaves its answer, when
    /// it comes, to nobody.
    pub async fn call(
        &self,
        method: &str,
        params: &impl Serialize,
    ) -> Result<Received, CallFailure> {
        let id = RequestId::Number(self.next.fetch_add(1, Ordering::Relaxed));
        let line = wire::request_line(&id, method, params).map_err(|error| {
            CallFailure::Unavailable(internal(&format!(
                "the params of `{method}` encode: {error}"
            )))
        })?;
        let (answer, answered) = oneshot::channel();
        if self.calls.send(Call { id, line, answer }).await.is_err() {
            return Err(self.end());
        }
        answered.await.unwrap_or_else(|_| Err(self.end()))
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

/// The instance's task.
struct Running {
    /// The instance's own owner, under the one it started on.
    owner: TaskOwner,
    /// Cancelled when its owner is stopped or [`PluginInstance::stop`] is
    /// called.
    stopping: CancellationToken,
    plugin: Plugin,
    scope: Scope,
    mailbox: mpsc::Receiver<Call>,
    ending: watch::Sender<Option<CallError>>,
}

/// The instance's state and the data beside it.
struct Instance {
    state: InstanceState,
    /// Calls made before its `hello` was answered.
    queued: Vec<Call>,
    /// Calls sent, by id, until answered.
    waiting: BTreeMap<RequestId, Call>,
    /// Whether its `hello` awaits its answer.
    hello_awaited: bool,
}

impl Instance {
    /// Whether an answer with `id` is awaited: its `hello`'s or a call's.
    fn awaits(&self, id: &RequestId) -> bool {
        (self.hello_awaited && *id == RequestId::Number(HELLO_ID)) || self.waiting.contains_key(id)
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

/// What ends an instance's loop.
enum Exit {
    /// Its owner was stopped, [`PluginInstance::stop`] called or every
    /// handle dropped: the event it was.
    Stopped(InstanceEvent),
    /// It ends with this reason, which its calls fail with.
    Failed(CallError),
}

impl Running {
    async fn run(mut self) {
        let started = PluginProcess::start(&self.plugin.launcher, &self.plugin.launch);
        let (mut process, Pipes { mut stdin, stdout }) = match started {
            Ok(started) => started,
            Err(error) => {
                let reason = internal(&format!("the Plugin's launcher did not start: {error}"));
                self.forget_folder().await;
                self.end(&reason, Vec::new(), BTreeMap::new());
                return;
            }
        };
        let mut instance = Instance {
            state: InstanceState::Starting,
            queued: Vec::new(),
            waiting: BTreeMap::new(),
            hello_awaited: true,
        };
        let exit = match self.hello_line() {
            Ok(line) => {
                write(&self.stopping, &mut stdin, &line).await;
                self.serve(&mut instance, &mut process, &mut stdin, stdout)
                    .await
            }
            Err(reason) => Exit::Failed(reason),
        };
        let reason = match exit {
            Exit::Stopped(event) => {
                // `run_ended` and `stop` stop the instance in every state.
                let _ = instance.apply(event);
                cancelled()
            }
            Exit::Failed(reason) => reason,
        };
        drop(stdin);
        if let Err(error) = process.stop(&self.owner).await {
            tracing::warn!(%error, "the instance's private temporary folder was not deleted");
        }
        self.end(&reason, instance.queued, instance.waiting);
    }

    /// The line of its `hello`.
    fn hello_line(&self) -> Result<Vec<u8>, CallError> {
        let params = PluginHelloParams {
            protocol: PLUGIN_PROTOCOL,
            product: self.plugin.product,
            run_id: self.scope.run_id().cloned(),
            network_level: self.plugin.manifest.network_level,
        };
        wire::request_line(&RequestId::Number(HELLO_ID), HELLO, &params)
            .map_err(|error| internal(&format!("`hello` encodes: {error}")))
    }

    /// Serves the instance until it ends.
    async fn serve(
        &mut self,
        instance: &mut Instance,
        process: &mut PluginProcess,
        stdin: &mut ChildStdin,
        mut stdout: ChildStdout,
    ) -> Exit {
        let mut framer = Framer::new(MAX_MESSAGE_BYTES);
        let (mut buffer, mut parsed) = (vec![0; READ_BYTES], Vec::new());
        let mut reading = true;
        loop {
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
                }
                ended = process.ended() => {
                    let reason = match ended {
                        Ok(Ended::ConfinementUnavailable) => code(
                            &CONFINEMENT_UNAVAILABLE,
                            "the host cannot confine the Plugin as its manifest declares",
                        ),
                        Ok(Ended::Exited(status)) => code(
                            &PLUGIN_CRASHED,
                            &format!("the Plugin exited with the status {status:?}"),
                        ),
                        Err(error) => code(
                            &PLUGIN_CRASHED,
                            &format!("the Plugin's end cannot be told: {error}"),
                        ),
                    };
                    // #837 counts the crashes and restarts the instance:
                    // until then it ends.
                    let _ = instance.apply(InstanceEvent::Crash(RecentCrashes::Fewer));
                    return Exit::Failed(reason);
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

    /// A call its caller made.
    async fn called(&self, instance: &mut Instance, stdin: &mut ChildStdin, call: Call) {
        match instance.apply(InstanceEvent::Call) {
            Ok(InstanceEffect::Queue) => instance.queued.push(call),
            Ok(InstanceEffect::Send) => send(&self.stopping, instance, stdin, call).await,
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

    /// A frame the Plugin sent: the exit it leads to, if any.
    async fn received(
        &self,
        instance: &mut Instance,
        stdin: &mut ChildStdin,
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
            Ok(Incoming::Request { call, .. } | Incoming::Notification(call)) => {
                stray(&format!(
                    "the Plugin called `{}`, which the host does not serve",
                    call.method()
                ));
                return None;
            }
            Err(error) => match error.id {
                Some(id) if instance.awaits(&id) => (
                    id,
                    Err(CallFailure::Malformed(Malformed {
                        detail: error.detail,
                    })),
                ),
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
        let Some(call) = instance.waiting.remove(&id) else {
            stray(&format!("the Plugin answered {id:?}, which no call awaits"));
            return None;
        };
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
        None
    }

    /// The Plugin's answer to `hello`: ready, or stopped when it differs
    /// from the approved manifest.
    async fn hello_answered(
        &self,
        instance: &mut Instance,
        stdin: &mut ChildStdin,
        answer: Result<Received, CallFailure>,
    ) -> Option<Exit> {
        let checked = answer
            .map_err(|failure| failure.to_string())
            .and_then(|received| {
                received
                    .read::<PluginHelloResult>()
                    .map_err(|error| format!("the answer to `hello` cannot be read: {error}"))
            })
            .map(|result| check_hello(PLUGIN_PROTOCOL, &self.plugin.manifest, &result));
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
                for call in std::mem::take(&mut instance.queued) {
                    send(&self.stopping, instance, stdin, call).await;
                }
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
    async fn forget_folder(&self) {
        let folder = self.plugin.launch.folder.clone();
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
    /// made later, fails with it.
    fn end(mut self, reason: &CallError, queued: Vec<Call>, waiting: BTreeMap<RequestId, Call>) {
        self.ending.send_replace(Some(reason.clone()));
        self.mailbox.close();
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
async fn send(
    stopping: &CancellationToken,
    instance: &mut Instance,
    stdin: &mut ChildStdin,
    call: Call,
) {
    write(stopping, stdin, &call.line).await;
    instance.waiting.insert(call.id.clone(), call);
}

/// Writes `line` to the Plugin, unless the instance is stopping. A write
/// the Plugin refuses fails its calls once its end shows.
async fn write(stopping: &CancellationToken, stdin: &mut ChildStdin, line: &[u8]) {
    let written = async {
        stdin.write_all(line).await?;
        stdin.flush().await
    };
    if let Some(Err(error)) = stopping.run_until_cancelled(written).await {
        tracing::debug!(%error, "the Plugin refused a message");
    }
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
