//! Where a command's call runs (ADRs 0024, 0088, 0098 and 0099): an
//! Attached engine in the CLI's own process, which it reaches over the
//! in-memory transport pair, serves this one command's call and stops with
//! it, stopping the Runs it started; or the Daemon of the executable's
//! major, which outlives the command. Either is opened with `hello` as the
//! form `cli`, needing the executable's Product (ADRs 0095 and 0099).
//!
//! A command whose value outlives it, a value-bound command or an opener,
//! needs a long-lived engine, so it reaches the Daemon, which the
//! executable starts when none runs, unless `--attached` is given; any
//! other runs on an Attached engine unless a Daemon runs, or one is
//! configured through `GOODFETCH_URL`, or `--daemon` is given. A Daemon
//! that refuses the command's `hello` by Product or protocol version and
//! cannot hand over ends it under `--daemon` with the refusal, whose `fix`
//! names this executable's `daemon restart`, and exit `11` whichever of the
//! two refused it (ticket #1183), and so does a command whose value
//! outlives it, which an Attached engine would stop with the command;
//! otherwise the command warns of the refusal and runs on an Attached
//! engine, as `engine: "auto"` does (ADR 0098). A Daemon that hands over
//! is waited for, and so is one that went away while it still stops. A
//! Daemon that cannot be reached fails the command with
//! `daemon_unreachable`, and a Connection to it that drops before the call
//! is answered with `connection_lost`. The token the Daemon takes never
//! leaves its `hello`.
//!
//! A followed call is granted credits as what it sends is printed (ADR
//! 0024). An interrupt (`SIGINT`, Ctrl-C, or `CTRL_BREAK_EVENT` on Windows)
//! or `SIGTERM` sends a cancellable call's `$/cancel` and waits for the
//! engine to stop its work, or, for a command that stops what it started,
//! such as a followed crawl, makes the call that stops it and waits for its
//! answer; a second one, or one during a call that cannot be cancelled,
//! ends the command at once (ADRs 0069, 0080 and 0093).
//!
//! The Daemon is found, opened with `hello` and looked for again by the one
//! Daemon client, [`Machine`]'s, which the `daemon` commands open their
//! Connections through too, listening for no interrupt, so `SIGINT` keeps
//! its default.
//!
//! Size: one concern, a command's Link to its engine: the choice of
//! engine, `hello`, and the one call loop that interleaves the call's
//! answers, its credits, the stream it sends and the interrupts, kept
//! whole so a change to one of them is checked against the others.
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Connection, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{CallError, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::{Operation, Product};
use goodfetch_protocol::connection::{
    CANCEL, CONNECTION_LOST, CancelParams, HELLO, HelloParams, Surface,
};
use goodfetch_protocol::daemon::{
    DAEMON_UNREACHABLE, DaemonEndpoint, DaemonToken, DaemonUnreachableDetails,
};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::jsonrpc::{
    ErrorObject, Message, Notification, Request, RequestId, Version,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::sync::mpsc;

use super::daemon::{Machine, NotOpened, Wanted};
use super::exit_codes;
use crate::runtime::{self, Threads};

/// The id of `hello`.
const HELLO_ID: i64 = 0;

/// The id of the command's call.
const CALL_ID: i64 = 1;

/// How many events or chunks a followed call may send beyond those printed.
const CREDITS: u32 = 64;

/// Which engine the command line asks for (ADR 0024).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// Neither flag: the Daemon when the command's value outlives it, or
    /// when one runs or is configured; an Attached engine otherwise, or
    /// when the Daemon refuses the command.
    #[default]
    Auto,
    /// `--attached`: an Attached engine in this process, whatever runs.
    Attached,
    /// `--daemon`: the Daemon, started when none runs; its refusal ends the
    /// command.
    Daemon,
}

// A hand-written command's `--attached` and `--daemon`, as every generated
// command takes them. Plain comments: a doc comment would become the help
// of every command that flattens it.
#[expect(
    missing_docs,
    reason = "a doc comment would become the help of every command that flattens it"
)]
#[derive(Clone, Copy, Debug, Default, clap::Args)]
pub struct EngineFlags {
    /// Runs the call on an Attached engine in this process, even when a
    /// Daemon runs.
    #[arg(long, conflicts_with = "daemon")]
    pub attached: bool,
    /// Runs the call on the Daemon of the major, starting it if none runs;
    /// its refusal ends the command.
    #[arg(long)]
    pub daemon: bool,
}

impl EngineFlags {
    /// The engine they ask for.
    pub fn choice(self) -> Choice {
        if self.attached {
            Choice::Attached
        } else if self.daemon {
            Choice::Daemon
        } else {
            Choice::Auto
        }
    }
}

/// What the command's call needs of its engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// Its value outlives the command, as a value-bound command's or an
    /// opener's does: a long-lived engine, the Daemon.
    Lasting,
    /// Nothing outlives the command.
    Passing,
}

/// The engine a command runs on: what its command line chose, and the
/// executable's machine, whose Daemon it reaches.
pub struct EngineChoice<'a> {
    /// The command line's choice.
    pub choice: Choice,
    /// What its call needs.
    pub lifetime: Lifetime,
    /// The executable's machine, through whose one Daemon client the call
    /// reaches the Daemon.
    pub machine: &'a Machine<'a>,
    /// Prints a warning, such as the refusal a command falls back from.
    pub warn: &'a dyn Fn(&str),
}

impl<'a> EngineChoice<'a> {
    /// The engine of a command whose value does not outlive it, as
    /// `choice` chooses: the Daemon when one runs or `--daemon` is given,
    /// else an Attached engine; refusals warned of through `warn`.
    pub fn passing(choice: Choice, machine: &'a Machine<'a>, warn: &'a dyn Fn(&str)) -> Self {
        Self {
            choice,
            lifetime: Lifetime::Passing,
            machine,
            warn,
        }
    }
}

/// Receives each notification a followed call sends, by its method and
/// params, and answers how many credits printing it used; `None` once the
/// command can print nothing more, as when stdout takes no more bytes: the
/// call is then cancelled, and the command ends once the engine answered.
pub type Follower<'a> = &'a mut dyn FnMut(&str, &Value) -> Option<u32>;

/// Receives each answer of a command's calls, and answers the call the
/// command makes next on the same Connection, such as the next page of a
/// call that answers page by page, or `None` when the answer is the last.
pub type Pager<'a> = &'a mut dyn FnMut(&Value) -> Option<Next>;

/// Answers, at the command's first interrupt, the call that stops what it
/// started in place of `$/cancel`, such as a crawl's `run.stop`, which
/// checkpoints its Run; `None` to cancel as any call is.
pub type Stopper<'a> = &'a mut dyn FnMut() -> Option<Next>;

/// A call a command makes after another on the same Connection.
pub struct Next {
    /// The operation.
    pub method: &'static str,
    /// Its params.
    pub params: Value,
    /// Whether `$/cancel` stops it.
    pub cancellable: bool,
    /// Whether it is a sequence whose notifications the command's follower
    /// receives, granted credits as it uses them.
    pub followed: bool,
}

/// The stream of values a call takes from the Client (ADR 0024): the
/// credits the engine grants the call with `$/credit` are spent on
/// notifications `method`, each listing in `field` the values ready, never
/// more than the credits left, `end` set with the last. The command keeps
/// listening for an interrupt and for the engine while it waits for one.
pub struct Input<'a> {
    /// The notification that carries the values, such as `$/seeds`.
    pub method: &'a str,
    /// The field of it that lists them.
    pub field: &'a str,
    /// Where they come from.
    pub values: Values<'a>,
}

/// Where the values of a call that takes a stream come from.
pub enum Values<'a> {
    /// Values the command holds already, sent in order.
    Held(&'a mut dyn Iterator<Item = Value>),
    /// Values read as they come, such as stdin's lines.
    Read(&'a mut ReadValues),
}

/// Values a reader that blocks, such as one of stdin, sends as it reads
/// them, on a tracked blocking thread the call starts (STYLE.md), into a
/// bounded channel: they end once the reader returns.
pub struct ReadValues {
    reader: Option<Reader>,
    receiver: Option<mpsc::Receiver<Value>>,
}

/// A reader of values: it sends each into the channel it is given, and
/// returns at their end, or once the channel's receiver is gone.
type Reader = Box<dyn FnOnce(mpsc::Sender<Value>) + Send>;

/// How many values read ahead a [`ReadValues`] holds before its reader
/// waits: a `$/seeds` batch's worth (`run.add_seeds`' credits).
const READ_AHEAD: usize = 256;

impl ReadValues {
    /// The values `reader` sends, read once the call starts.
    pub fn new(reader: impl FnOnce(mpsc::Sender<Value>) + Send + 'static) -> Self {
        Self {
            reader: Some(Box::new(reader)),
            receiver: None,
        }
    }

    /// Starts the reader on a blocking thread of `owner`, once.
    fn start(&mut self, owner: &TaskOwner) {
        let Some(reader) = self.reader.take() else {
            return;
        };
        let (sender, receiver) = mpsc::channel(READ_AHEAD);
        self.receiver = Some(receiver);
        let reading = owner.child();
        owner.spawn(async move { reading.blocking(move || reader(sender)).await });
    }
}

impl Values<'_> {
    /// The values ready, at least one and at most `most`, waiting for the
    /// first; and whether they are the last.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no value.
    async fn ready(&mut self, most: usize) -> (Vec<Value>, bool) {
        match self {
            Self::Held(values) => {
                let taken: Vec<Value> = (&mut **values).take(most).collect();
                let end = taken.len() < most;
                (taken, end)
            }
            Self::Read(read) => {
                let Some(receiver) = read.receiver.as_mut() else {
                    return (Vec::new(), true);
                };
                let Some(first) = receiver.recv().await else {
                    return (Vec::new(), true);
                };
                let mut taken = vec![first];
                while taken.len() < most {
                    match receiver.try_recv() {
                        Ok(value) => taken.push(value),
                        Err(mpsc::error::TryRecvError::Empty) => return (taken, false),
                        Err(mpsc::error::TryRecvError::Disconnected) => return (taken, true),
                    }
                }
                (taken, false)
            }
        }
    }
}

/// The one call a command makes, or, for a call that answers page by page,
/// the calls of its pages, on one Connection.
pub struct CommandCall<'a> {
    /// The operation.
    pub method: &'a str,
    /// Its params.
    pub params: Value,
    /// Whether `$/cancel` stops it.
    pub cancellable: bool,
    /// For a sequence, what receives what it sends, granted credits as it
    /// uses them.
    pub follower: Option<Follower<'a>>,
    /// For a command that makes more than one call, such as one that asks
    /// for page after page under `--follow`, what asks for the next.
    pub pager: Option<Pager<'a>>,
    /// For a call that takes a stream of values, where they come from.
    pub input: Option<Input<'a>>,
    /// For a command that stops what it started at its first interrupt,
    /// the call that stops it.
    pub stopper: Option<Stopper<'a>>,
}

impl<'a> CommandCall<'a> {
    /// The one call of `operation` with `params`, which nothing follows.
    pub fn of(operation: &'a Operation, params: Value) -> Self {
        Self {
            method: operation.name,
            params,
            cancellable: operation.cancellable,
            follower: None,
            pager: None,
            input: None,
            stopper: None,
        }
    }
}

/// How a call ended.
pub enum Answer {
    /// Its result.
    Result(Value),
    /// The engine's error, or its refusal of `hello`.
    Error(ErrorObject),
    /// Under `--daemon`, the Daemon's refusal of `hello` by Product or by
    /// protocol version, its `fix` naming this executable's `daemon
    /// restart`: the command exits [`exit_codes::DAEMON_REFUSED`].
    Refused(ErrorObject),
    /// No call was made, for the reason its Error code says: no Daemon
    /// could be reached (`daemon_unreachable`), a `GOODFETCH_URL` or
    /// `GOODFETCH_TOKEN` it cannot take (`setting_invalid`), or an Attached
    /// engine that took no Connection, a bug (`internal_error`). It prints
    /// as an error does, naming no operation.
    Unanswered(ErrorObject),
    /// The command was interrupted, or sent `SIGTERM`, and its call
    /// cancelled when it could be.
    Interrupted(Interrupt),
}

impl Answer {
    /// Its result, or how the call failed.
    pub fn into_result(self) -> Result<Value, Failed> {
        match self {
            Self::Result(value) => Ok(value),
            Self::Error(error) => Err(Failed::Error(error)),
            Self::Refused(error) => Err(Failed::Refused(error)),
            Self::Unanswered(error) => Err(Failed::Unanswered(error)),
            Self::Interrupted(interrupt) => Err(Failed::Interrupted(interrupt)),
        }
    }
}

/// How a call that answered no result failed, as [`Answer`] says it.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Failed {
    /// The engine's error, or its refusal of `hello`.
    #[error("{}", .0.message)]
    Error(ErrorObject),
    /// Under `--daemon`, the Daemon's refusal of `hello` by Product or by
    /// protocol version.
    #[error("{}", .0.message)]
    Refused(ErrorObject),
    /// No call was made, for the reason its Error code says.
    #[error("{}", .0.message)]
    Unanswered(ErrorObject),
    /// The command was interrupted, or sent `SIGTERM`.
    #[error("the command was interrupted")]
    Interrupted(Interrupt),
}

/// What stopped a command before its call was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupt {
    /// `SIGINT` (Ctrl-C), or `CTRL_BREAK_EVENT` on Windows.
    Interrupted,
    /// `SIGTERM`, as a service manager or `kill` sends.
    Terminated,
}

impl Interrupt {
    /// Its row of the exit-code table: `130` or `143`, the shell's `128`
    /// plus the signal's number.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Interrupted => exit_codes::INTERRUPTED,
            Self::Terminated => exit_codes::TERMINATED,
        }
    }
}

/// Calls `call` on the engine `reach` chooses: the Daemon, or an Attached
/// engine in this process, `engine` on `dependencies`; either opened by a
/// Client that needs `needs`, on a runtime whose threads the machine's
/// settings choose.
///
/// # Panics
///
/// If Tokio cannot build its runtime.
pub fn call(
    engine: &Engine,
    dependencies: Dependencies,
    needs: Product,
    reach: &EngineChoice<'_>,
    call: CommandCall<'_>,
) -> Answer {
    // Settings it cannot read leave their defaults here: the engine, or
    // the Daemon, refuses the call with their Error code.
    let threads = Threads::of(reach.machine.settings().ok());
    let start = match (reach.choice, reach.lifetime) {
        (Choice::Attached, _) => {
            return call_attached(engine, dependencies, needs, threads, call);
        }
        (Choice::Daemon, _) | (Choice::Auto, Lifetime::Lasting) => true,
        (Choice::Auto, Lifetime::Passing) => false,
    };
    let reads = call.reads();
    let runtime = runtime::multi_thread(threads);
    let wanted = Wanted {
        start,
        remote: true,
        needs,
    };
    let (mut link, found) = match reach.machine.open_daemon(
        &runtime,
        &wanted,
        Interrupts::listening,
    ) {
        Ok(Some(opened)) => opened,
        Ok(None) => {
            drop(runtime);
            return call_attached(engine, dependencies, needs, threads, call);
        }
        Err(NotOpened::Interrupted(interrupt)) => return Answer::Interrupted(interrupt),
        Err(NotOpened::Refused(refusal)) => return Answer::Error(refusal),
        Err(NotOpened::Failed(failure)) => return Answer::Unanswered(failure.into_error()),
        Err(NotOpened::RefusedByProductOrVersion(refusal)) => {
            // An Attached engine stops with the command, so a value
            // that outlives it cannot run on one.
            if reach.choice == Choice::Daemon || reach.lifetime == Lifetime::Lasting {
                return Answer::Refused(refusal);
            }
            (reach.warn)(&format!(
                "the Daemon refused this command, which runs on an Attached engine instead: {} {}",
                refusal.message,
                refusal.data.details["fix"].as_str().unwrap_or_default()
            ));
            drop(runtime);
            return call_attached(engine, dependencies, needs, threads, call);
        }
    };
    let answer = runtime.block_on(async {
        let answer = link.call(call).await;
        link.close().await;
        answer
    });
    end(runtime, reads);
    match answer {
        // The call in flight, the command's own or a later one on the same
        // Connection, is the one whose answer is lost.
        Answer::Unanswered(_) => Answer::Error(connection_lost(&found.endpoint, &link.method)),
        answer => answer,
    }
}

/// The failure of a command that reached no Daemon, as `message` says,
/// and what to do, `fix`: `daemon_unreachable`, which exits its group's
/// row, `8`.
pub fn unreachable(message: String, fix: &str) -> ErrorObject {
    let details = DaemonUnreachableDetails {
        fix: fix.to_owned(),
    };
    CallError::new(&DAEMON_UNREACHABLE, message, &details).to_error_object()
}

/// Calls `call` on an Attached engine in this process, `engine` on
/// `dependencies` and a runtime of `threads`, opened by a Client that
/// needs `needs`.
///
/// # Panics
///
/// If Tokio cannot build its runtime.
fn call_attached(
    engine: &Engine,
    dependencies: Dependencies,
    needs: Product,
    threads: Threads,
    call: CommandCall<'_>,
) -> Answer {
    let reads = call.reads();
    let runtime = runtime::multi_thread(threads);
    let hello = hello(engine.identity(), needs, None);
    let (network, dialer) = transport::in_memory();
    let answer = runtime.block_on(async {
        let client = async {
            match dialer.connect().await {
                Ok(connection) => exchange(connection, &hello, call).await,
                Err(error) => Answer::Unanswered(exit_codes::internal(format!(
                    "The Attached engine took no Connection: {error}."
                ))),
            }
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        match (served, answer) {
            (Err(error), Answer::Unanswered(_)) => Answer::Unanswered(exit_codes::internal(
                format!("The Attached engine failed: {error}."),
            )),
            (_, answer) => answer,
        }
    });
    end(runtime, reads);
    answer
}

/// Ends the runtime of a command's call once the call ended: waiting for
/// its blocking work, unless the call `reads` values on a blocking thread
/// that only new input wakes, such as stdin's, which an ended call no
/// longer waits for.
fn end(runtime: tokio::runtime::Runtime, reads: bool) {
    if reads {
        runtime.shutdown_background();
    } else {
        drop(runtime);
    }
}

impl CommandCall<'_> {
    /// Whether its values are read as they come, on a blocking thread.
    fn reads(&self) -> bool {
        matches!(
            self.input,
            Some(Input {
                values: Values::Read(_),
                ..
            })
        )
    }
}

/// The `hello` of a Client of the executable `identity` names that needs
/// `needs`, carrying `token`, the Daemon's, which an Attached engine takes
/// none of.
pub(crate) fn hello(
    identity: &Identity,
    needs: Product,
    token: Option<&DaemonToken>,
) -> HelloParams {
    HelloParams {
        token: token.map(|token| token.expose().to_owned()),
        needs: Some(needs),
        ..HelloParams::new(Surface::Cli, identity.product_version.clone())
    }
}

/// The failure of a call whose Connection to the Daemon at `endpoint`
/// dropped before `method` was answered: `connection_lost`, which exits
/// its group's row, `8`.
pub fn connection_lost(endpoint: &DaemonEndpoint, method: &str) -> ErrorObject {
    let details = json!({
        "fix": "Check whether the call took effect, then run the command again; `daemon status` shows whether the Daemon still runs.",
    });
    CallError::new(
        &CONNECTION_LOST,
        format!(
            "The Connection to the Daemon at {endpoint} dropped before it answered `{method}`, so whether the call took effect is unknown."
        ),
        &details,
    )
    .to_error_object()
}

/// Opens `connection` with `hello`, then makes the one call, and closes
/// the Connection once it is answered, which stops the Attached engine.
async fn exchange(connection: Connection, hello: &HelloParams, call: CommandCall<'_>) -> Answer {
    let mut link = Link::of(connection, Interrupts::listening());
    let answer = match link.hello(hello).await {
        Ok(()) => link.call(call).await,
        Err(Opening::Interrupted(interrupt)) => Answer::Interrupted(interrupt),
        Err(Opening::Refused(error) | Opening::HandOver(error)) => Answer::Error(error),
        Err(Opening::Gone | Opening::Unreachable) => closed(HELLO),
    };
    link.close().await;
    // The call was made: an Attached engine that ended it unanswered fails
    // it, as a bug does.
    match answer {
        Answer::Unanswered(error) => Answer::Error(error),
        answer => answer,
    }
}

/// Why opening a Connection with `hello` failed.
pub(crate) enum Opening {
    /// Nothing answered at the endpoint.
    Unreachable,
    /// The engine closed the Connection before it answered.
    Gone,
    /// The engine refused this Client.
    Refused(ErrorObject),
    /// The Daemon refused this Client and hands over to its major's pick.
    HandOver(ErrorObject),
    /// The command was interrupted first.
    Interrupted(Interrupt),
}

/// The Client's end of the Connection: the one Daemon client, which a
/// generated command and the `daemon` commands both open.
pub(crate) struct Link {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    /// The operation of the call in flight, or last made.
    method: String,
    /// The signals that interrupt the command, listened for from the
    /// Connection's opening to its end; none for a Link that leaves them
    /// to their default, as a `daemon` command's does.
    interrupts: Interrupts,
}

impl Link {
    /// The Client's end of `connection`, listening for `interrupts`; called
    /// on the command's runtime.
    fn of(connection: Connection, interrupts: Interrupts) -> Self {
        let (reader, writer) = connection.split();
        Self {
            lines: BufReader::new(reader).lines(),
            writer,
            method: HELLO.to_owned(),
            interrupts,
        }
    }

    /// A Connection to the Daemon at `endpoint`, opened with `hello`; once
    /// connected, it listens for the interrupts `interrupts` gives, on the
    /// command's runtime.
    pub(crate) async fn open(
        endpoint: &DaemonEndpoint,
        hello: &HelloParams,
        interrupts: fn() -> Interrupts,
    ) -> Result<Self, Opening> {
        let connection = transport::connect_daemon(endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut link = Self::of(connection, interrupts());
        match link.hello(hello).await {
            Ok(()) => Ok(link),
            Err(failure) => {
                link.close().await;
                Err(failure)
            }
        }
    }

    /// Closes the Connection.
    pub(crate) async fn close(&mut self) {
        let _ = self.writer.shutdown().await;
    }

    /// Calls `method` with `params` as the call `id`, and reads its reply:
    /// its result, its error, or `None` once the engine closed the
    /// Connection. An error whose id is null, the engine's answer to a
    /// message it could not read, is its reply, as it is to a command's
    /// call; events and other replies are not.
    pub(crate) async fn request(
        &mut self,
        id: i64,
        method: &str,
        params: Value,
    ) -> Result<Value, Option<ErrorObject>> {
        method.clone_into(&mut self.method);
        if !self.send(&request(id, method, params)).await {
            return Err(None);
        }
        let id = RequestId::Number(id);
        loop {
            match next(&mut self.lines).await {
                None => return Err(None),
                Some(Message::Response(response)) if response.id == id => {
                    return Ok(response.result);
                }
                Some(Message::Error(failure))
                    if failure.id.is_none() || failure.id.as_ref() == Some(&id) =>
                {
                    return Err(Some(failure.error));
                }
                Some(_) => {}
            }
        }
    }

    /// Says `hello` and reads its answer.
    async fn hello(&mut self, hello: &HelloParams) -> Result<(), Opening> {
        let hello = serde_json::to_value(hello).expect("invariant: hello encodes");
        if !self.send(&request(HELLO_ID, HELLO, hello)).await {
            return Err(Opening::Gone);
        }
        loop {
            tokio::select! {
                interrupt = self.interrupts.next() => return Err(Opening::Interrupted(interrupt)),
                message = next(&mut self.lines) => match message {
                    None => return Err(Opening::Gone),
                    Some(Message::Response(response)) if response.id == RequestId::Number(HELLO_ID) => return Ok(()),
                    // An error whose id is null answers a message the
                    // engine could not read, which can only be `hello`.
                    Some(Message::Error(failure)) if failure.id.is_none() || failure.id == Some(RequestId::Number(HELLO_ID)) => {
                        let handover = failure.error.data.details.get("handover").and_then(Value::as_bool);
                        return Err(if handover == Some(true) {
                            Opening::HandOver(failure.error)
                        } else {
                            Opening::Refused(failure.error)
                        });
                    }
                    Some(_) => {}
                },
            }
        }
    }

    /// Makes the call on the opened Connection and reads its answer; for a
    /// command that makes more than one call, makes each next call the
    /// pager asks for, and answers the last one's answer. At its first
    /// interrupt, a command whose stopper gives a call makes it and ends
    /// once it is answered; any other cancels its call when it can. A
    /// follower that can print nothing more has its call cancelled, and the
    /// command then ends with whatever the engine answers. An error whose
    /// id is null, the engine's answer to a message it could not read, such
    /// as one past its size limit, ends the command with that error: no
    /// answer to the call in flight follows it.
    #[expect(
        clippy::too_many_lines,
        reason = "one loop reads every message of the command's calls; splitting it would pass its state between functions"
    )]
    async fn call(&mut self, mut call: CommandCall<'_>) -> Answer {
        let mut id = CALL_ID;
        let mut method = call.method;
        call.method.clone_into(&mut self.method);
        let mut cancellable = call.cancellable;
        let params = std::mem::take(&mut call.params);
        if !self.send(&request(id, method, params)).await {
            return closed(method);
        }
        // Once the follower printed nothing more, what the call sends is
        // no longer given it.
        let mut abandoned = false;
        // A call that takes a stream grants the credits itself, and a
        // command that makes more than one call credits only those its
        // pager says are followed.
        let mut credited = call.follower.is_some() && call.input.is_none() && call.pager.is_none();
        if credited && !self.credit(id, CREDITS).await {
            return closed(method);
        }
        let mut cancelled = None;
        // The stopping call, once the first interrupt made one.
        let mut stopping: Option<(i64, Interrupt)> = None;
        // The credits the stream's call granted that its values have not
        // spent yet, and whether its last values were sent.
        let mut owed: usize = 0;
        let mut input_ended = false;
        let readers = TaskOwner::root(BlockingPolicy::Threads);
        if let Some(Input {
            values: Values::Read(read),
            ..
        }) = call.input.as_mut()
        {
            read.start(&readers);
        }
        loop {
            let wants_values =
                owed > 0 && !input_ended && cancelled.is_none() && stopping.is_none();
            tokio::select! {
                interrupt_now = self.interrupts.next() => {
                    if cancelled.is_some() || stopping.is_some() {
                        return Answer::Interrupted(interrupt_now);
                    }
                    if let Some(stop) = call.stopper.as_mut().and_then(|stopper| stopper()) {
                        let stop_id = id + 1;
                        if !self.send(&request(stop_id, stop.method, stop.params)).await {
                            return Answer::Interrupted(interrupt_now);
                        }
                        stopping = Some((stop_id, interrupt_now));
                        continue;
                    }
                    if !cancellable {
                        return Answer::Interrupted(interrupt_now);
                    }
                    let cancel = notification(CANCEL, &CancelParams { id: RequestId::Number(id) });
                    if !self.send(&cancel).await {
                        return Answer::Interrupted(interrupt_now);
                    }
                    cancelled = Some(interrupt_now);
                }
                ready = ready_values(call.input.as_mut(), owed), if wants_values => {
                    let (values, end) = ready;
                    owed = owed.saturating_sub(values.len());
                    input_ended = end;
                    let Some(input) = call.input.as_ref() else { continue };
                    let mut batch = Map::new();
                    batch.insert("id".to_owned(), Value::from(id));
                    batch.insert(input.field.to_owned(), Value::Array(values));
                    batch.insert("end".to_owned(), Value::Bool(end));
                    if !self.send(&notification(input.method, &batch)).await {
                        return closed(method);
                    }
                }
                message = next(&mut self.lines) => match message {
                    None => return closed(method),
                    // The engine could not read a message the command
                    // sent, so the call it belongs to is never answered.
                    Some(Message::Error(failure)) if failure.id.is_none() => {
                        return match stopping.map(|(_, interrupt)| interrupt).or(cancelled) {
                            Some(interrupt) => Answer::Interrupted(interrupt),
                            None => Answer::Error(failure.error),
                        };
                    }
                    // The stopping call's answer, whatever it is, ends the
                    // command as interrupted.
                    Some(Message::Response(response)) if stopping.is_some_and(|(stop_id, _)| response.id == RequestId::Number(stop_id)) => {
                        return Answer::Interrupted(stopping.map_or(Interrupt::Interrupted, |(_, interrupt)| interrupt));
                    }
                    Some(Message::Error(failure)) if stopping.is_some_and(|(stop_id, _)| failure.id == Some(RequestId::Number(stop_id))) => {
                        return Answer::Interrupted(stopping.map_or(Interrupt::Interrupted, |(_, interrupt)| interrupt));
                    }
                    // While it stops, the command's own calls are no longer
                    // read.
                    Some(_) if stopping.is_some() => {}
                    Some(Message::Response(response)) if response.id == RequestId::Number(id) => {
                        if let Some(interrupt) = cancelled {
                            return Answer::Interrupted(interrupt);
                        }
                        let next = call.pager.as_mut().and_then(|pager| pager(&response.result));
                        let Some(next) = next else {
                            return Answer::Result(response.result);
                        };
                        id += 1;
                        method = next.method;
                        next.method.clone_into(&mut self.method);
                        cancellable = next.cancellable;
                        if !self.send(&request(id, method, next.params)).await {
                            return closed(method);
                        }
                        credited = next.followed && call.follower.is_some();
                        if credited && !self.credit(id, CREDITS).await {
                            return closed(method);
                        }
                    }
                    Some(Message::Error(failure)) if failure.id == Some(RequestId::Number(id)) => {
                        return cancelled.map_or(Answer::Error(failure.error), Answer::Interrupted);
                    }
                    Some(Message::Notification(sent)) => {
                        let params = sent.params.unwrap_or(Value::Null);
                        if params["id"] != id {
                            continue;
                        }
                        if call.input.is_some() && sent.method == CREDIT {
                            let credits = params["credits"].as_u64().unwrap_or(0);
                            owed = owed.saturating_add(usize::try_from(credits).unwrap_or(usize::MAX));
                            continue;
                        }
                        if abandoned {
                            continue;
                        }
                        let Some(follower) = call.follower.as_mut() else { continue };
                        if let Some(used) = follower(&sent.method, &params) {
                            if credited && used > 0 && !self.credit(id, used).await {
                                return closed(method);
                            }
                        } else {
                            // It prints nothing more: the call stops, and
                            // its answer ends the command.
                            abandoned = true;
                            let cancel = notification(CANCEL, &CancelParams { id: RequestId::Number(id) });
                            if !cancellable || !self.send(&cancel).await {
                                return closed(method);
                            }
                        }
                    }
                    Some(_) => {}
                },
            }
        }
    }

    /// Grants the call `id` `credits` more events or chunks.
    async fn credit(&mut self, id: i64, credits: u32) -> bool {
        let id = RequestId::Number(id);
        self.send(&notification(CREDIT, &CreditParams { id, credits }))
            .await
    }

    /// Writes `message` as one line; `false` when the engine is gone.
    async fn send(&mut self, message: &impl Serialize) -> bool {
        let mut line = serde_json::to_vec(message).expect("invariant: a message encodes");
        record_wire(SENT, &line);
        line.push(b'\n');
        self.writer.write_all(&line).await.is_ok() && self.writer.flush().await.is_ok()
    }
}

/// The next message the engine sends on `lines`, or `None` once it closed
/// the Connection; a line that is no message is passed over.
///
/// # Cancel safety
///
/// Dropping the future loses no message: `Lines::next_line` keeps a line
/// it read part of.
async fn next(lines: &mut Lines<BufReader<ConnectionReader>>) -> Option<Message> {
    loop {
        let line = lines.next_line().await.ok()??;
        if let Ok(message) = serde_json::from_str(&line) {
            record_wire(RECEIVED, line.as_bytes());
            return Some(message);
        }
    }
}

/// The values of `input` ready to send, at most `most`, and whether they
/// are the last; never for a call that takes none.
async fn ready_values(input: Option<&mut Input<'_>>, most: usize) -> (Vec<Value>, bool) {
    match input {
        Some(input) => input.values.ready(most).await,
        None => std::future::pending().await,
    }
}

/// The `tracing` target of the records of the messages a command's
/// Connection carries, from which a test counts them (ADR 0071's protocol
/// counts).
pub const WIRE_TARGET: &str = "goodfetch::cli::wire";

/// The direction of a message the command sent.
const SENT: &str = "sent";

/// The direction of a message the command received.
const RECEIVED: &str = "received";

/// Records one message of the Connection, at `trace`: its direction, its
/// kind (`request`, `notification`, `response` or `error`), its method
/// when it has one, and its bytes. It never records the message's content,
/// which may hold the Daemon's token.
fn record_wire(direction: &str, line: &[u8]) {
    if !tracing::enabled!(target: WIRE_TARGET, tracing::Level::TRACE) {
        return;
    }
    let Ok(message) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let method = message["method"].as_str().unwrap_or_default();
    let kind = match (method.is_empty(), message.get("id").is_some()) {
        (false, true) => "request",
        (false, false) => "notification",
        (true, _) if message.get("error").is_some() => "error",
        (true, _) => "response",
    };
    tracing::trace!(
        target: WIRE_TARGET,
        direction,
        kind,
        method,
        bytes = line.len(),
        "message"
    );
}

/// The signals that interrupt a command (`SIGINT`, or Ctrl-C and
/// `CTRL_BREAK_EVENT` on Windows) or end it (`SIGTERM`), each listened for
/// from its creation on, so one that arrives while the command handles the
/// last, such as a second interrupt while a stop is sent, waits for the
/// next look; a signal the CLI cannot listen for never arrives.
pub(crate) struct Interrupts {
    #[cfg(unix)]
    interrupt: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
    #[cfg(windows)]
    ctrl_c: Option<tokio::signal::windows::CtrlC>,
    /// `CTRL_BREAK_EVENT`, which a process group gets where Unix sends
    /// `SIGINT` (ADR 0093).
    #[cfg(windows)]
    ctrl_break: Option<tokio::signal::windows::CtrlBreak>,
}

impl Interrupts {
    /// None: the signals keep their default, which ends the process, as
    /// they do for a `daemon` command, which installs no listener.
    pub(crate) fn none() -> Self {
        Self {
            #[cfg(unix)]
            interrupt: None,
            #[cfg(unix)]
            terminate: None,
            #[cfg(windows)]
            ctrl_c: None,
            #[cfg(windows)]
            ctrl_break: None,
        }
    }

    /// Listens for them; called on the command's runtime.
    #[cfg(unix)]
    fn listening() -> Self {
        use tokio::signal::unix::{SignalKind, signal};
        Self {
            interrupt: signal(SignalKind::interrupt()).ok(),
            terminate: signal(SignalKind::terminate()).ok(),
        }
    }

    /// Listens for them; called on the command's runtime.
    #[cfg(windows)]
    fn listening() -> Self {
        Self {
            ctrl_c: tokio::signal::windows::ctrl_c().ok(),
            ctrl_break: tokio::signal::windows::ctrl_break().ok(),
        }
    }

    /// The next one, once it arrives.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no signal.
    #[cfg(unix)]
    async fn next(&mut self) -> Interrupt {
        tokio::select! {
            () = received(self.interrupt.as_mut().map(tokio::signal::unix::Signal::recv)) => Interrupt::Interrupted,
            () = received(self.terminate.as_mut().map(tokio::signal::unix::Signal::recv)) => Interrupt::Terminated,
        }
    }

    /// The next one, once it arrives.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no signal.
    #[cfg(windows)]
    async fn next(&mut self) -> Interrupt {
        tokio::select! {
            () = received(self.ctrl_c.as_mut().map(tokio::signal::windows::CtrlC::recv)) => Interrupt::Interrupted,
            () = received(self.ctrl_break.as_mut().map(tokio::signal::windows::CtrlBreak::recv)) => Interrupt::Interrupted,
        }
    }
}

/// Resolves once the signal `listening` waits for arrives; never when the
/// CLI cannot listen for it.
async fn received(listening: Option<impl Future<Output = Option<()>>>) {
    let arrived = match listening {
        Some(listening) => listening.await.is_some(),
        None => false,
    };
    if !arrived {
        std::future::pending::<()>().await;
    }
}

fn request(id: i64, method: &str, params: Value) -> Request {
    Request {
        jsonrpc: Version,
        id: RequestId::Number(id),
        method: method.to_owned(),
        params: Some(params),
    }
}

fn notification(method: &str, params: &impl Serialize) -> Notification {
    Notification {
        jsonrpc: Version,
        method: method.to_owned(),
        params: Some(serde_json::to_value(params).expect("invariant: params encode")),
    }
}

/// The answer of a call whose Connection ended before `method` was
/// answered: a Daemon's call fails with `connection_lost`; an Attached
/// engine never ends it first but by a bug.
fn closed(method: &str) -> Answer {
    Answer::Unanswered(exit_codes::internal(format!(
        "The engine closed the Connection before it answered `{method}`."
    )))
}
