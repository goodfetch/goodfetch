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
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI builds its runtime before and outside any async worker, as main does (ADR 0060)"
)]

use goodfetch_engine::transport::{self, Connection, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    CANCEL, CONNECTION_LOST, CancelParams, HELLO, HelloParams, NOT_IN_PRODUCT,
    PROTOCOL_VERSION_UNSUPPORTED, Surface,
};
use goodfetch_protocol::daemon::{
    DAEMON_UNREACHABLE, DaemonEndpoint, DaemonToken, DaemonUnreachableDetails,
};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, Message, Notification, Request, RequestId, Version,
};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

use crate::exit_codes;

/// The id of `hello`.
const HELLO_ID: i64 = 0;

/// The id of the command's call.
const CALL_ID: i64 = 1;

/// How many events or chunks a followed call may send beyond those printed.
const CREDITS: u32 = 64;

/// How many times a command looks for the Daemon again when the one it
/// found went away, or handed over, before it gives up.
const ATTEMPTS: usize = 3;

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

/// Where a Client reaches a Daemon and the token its `hello` carries.
pub struct Reached {
    /// The Daemon's endpoint.
    pub endpoint: DaemonEndpoint,
    /// The token its `hello` carries, which nothing prints.
    pub token: DaemonToken,
    /// Whether it is a remote Daemon `GOODFETCH_URL` names, which no
    /// command starts, waits for or falls back from.
    pub remote: bool,
}

/// The executable's side of its major's Daemon, which this crate cannot
/// know (ADRs 0024 and 0098): where it runs, which executable starts as
/// it, and how this executable restarts it.
pub trait Daemons {
    /// The Daemon a command reaches: the remote one `GOODFETCH_URL` names;
    /// else the running Daemon of the major, or, when `start` and none
    /// runs, its major's pick, which this starts; `None` when none runs
    /// and `start` is false.
    ///
    /// # Errors
    ///
    /// What kept it from finding or starting one, with its Error code: a
    /// `GOODFETCH_URL` or `GOODFETCH_TOKEN` it cannot take is
    /// `setting_invalid`, and a Daemon it cannot read or start
    /// `daemon_unreachable`.
    fn find(&self, start: bool) -> Result<Option<Reached>, ErrorObject>;

    /// Waits until a Daemon that refused the command and hands over has
    /// started its major's pick.
    ///
    /// # Errors
    ///
    /// What kept it from waiting, with its Error code.
    fn handed_over(&self) -> Result<(), ErrorObject>;

    /// Waits, at most as long as a Daemon's stop takes, until the Daemon
    /// `found` of the major, which went away while the command connected,
    /// has stopped: a stopping Daemon closes its listener first and stops
    /// its Runs before it lets its runtime file go.
    ///
    /// # Errors
    ///
    /// What kept it from waiting, with its Error code.
    fn went_away(&self, found: &Reached) -> Result<(), ErrorObject>;

    /// The `fix` of a refusal by Product or protocol version, whose own
    /// `fix` is `given`: ADR 0098's, naming this executable's `daemon
    /// restart` by its path.
    fn refusal_fix(&self, given: &str) -> String;
}

/// The engine a command runs on: what its command line chose, and the
/// executable's Daemon.
pub struct EngineChoice<'a> {
    /// The command line's choice.
    pub choice: Choice,
    /// What its call needs.
    pub lifetime: Lifetime,
    /// The executable's Daemon.
    pub daemons: &'a dyn Daemons,
    /// Prints a warning, such as the refusal a command falls back from.
    pub warn: &'a dyn Fn(&str),
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

/// The stream of values a call takes from the Client (ADR 0024): each
/// `$/credit` the engine grants the call is answered by one notification
/// `method` listing up to that many values in `field`, `end` set with the
/// last.
pub struct Input<'a> {
    /// The notification that carries the values, such as `$/seeds`.
    pub method: &'a str,
    /// The field of it that lists them.
    pub field: &'a str,
    /// Gives at most as many values as it is passed, and whether they are
    /// the last.
    pub next: &'a mut dyn FnMut(u32) -> (Vec<Value>, bool),
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
/// Client that needs `needs`.
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
    let start = match (reach.choice, reach.lifetime) {
        (Choice::Attached, _) => return call_attached(engine, dependencies, needs, call),
        (Choice::Daemon, _) | (Choice::Auto, Lifetime::Lasting) => true,
        (Choice::Auto, Lifetime::Passing) => false,
    };
    let runtime = runtime();
    for _ in 0..ATTEMPTS {
        // The executable finds and starts the Daemon outside the runtime,
        // which its own blocking work would stall.
        let found = match reach.daemons.find(start) {
            Ok(Some(found)) => found,
            Ok(None) => return call_attached(engine, dependencies, needs, call),
            Err(problem) => return Answer::Unanswered(problem),
        };
        let hello = HelloParams {
            token: Some(found.token.expose().to_owned()),
            ..hello(engine, needs)
        };
        match runtime.block_on(Link::open(&found.endpoint, &hello)) {
            Ok(mut link) => {
                let answer = runtime.block_on(async {
                    let answer = link.call(call).await;
                    let _ = link.writer.shutdown().await;
                    answer
                });
                return match answer {
                    // The call in flight, the command's own or a later one
                    // on the same Connection, is the one whose answer is
                    // lost.
                    Answer::Unanswered(_) => {
                        Answer::Error(connection_lost(&found.endpoint, &link.method))
                    }
                    answer => answer,
                };
            }
            Err(Opening::Interrupted(interrupt)) => return Answer::Interrupted(interrupt),
            Err(Opening::Refused(mut refusal)) => {
                let by_product_or_version =
                    [NOT_IN_PRODUCT.name, PROTOCOL_VERSION_UNSUPPORTED.name]
                        .contains(&refusal.data.code.as_str());
                if found.remote || !by_product_or_version {
                    return Answer::Error(refusal);
                }
                let given = refusal.data.details.get("fix").and_then(Value::as_str);
                let fix = reach.daemons.refusal_fix(given.unwrap_or_default());
                refusal
                    .data
                    .details
                    .insert("fix".to_owned(), Value::from(fix));
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
                return call_attached(engine, dependencies, needs, call);
            }
            // It hands over: once the pick runs, look again.
            Err(Opening::HandOver(refusal)) => {
                if found.remote {
                    return Answer::Error(refusal);
                }
                if let Err(problem) = reach.daemons.handed_over() {
                    return Answer::Unanswered(problem);
                }
            }
            Err(Opening::Gone) if found.remote => {
                return Answer::Unanswered(unreachable(
                    format!(
                        "The Daemon at {} closed the Connection before it answered `hello`, as it does for a token it does not take.",
                        found.endpoint
                    ),
                    "Set `GOODFETCH_TOKEN` to the token the Daemon `GOODFETCH_URL` names takes, then run the command again.",
                ));
            }
            Err(Opening::Unreachable) if found.remote => {
                return Answer::Unanswered(unreachable(
                    format!(
                        "Nothing answers at {}, the Daemon `GOODFETCH_URL` names.",
                        found.endpoint
                    ),
                    "Start the Daemon `GOODFETCH_URL` names, or unset `GOODFETCH_URL` to use this machine's, then run the command again.",
                ));
            }
            // It went away in between: once it stopped, look again.
            Err(Opening::Gone | Opening::Unreachable) => {
                if let Err(problem) = reach.daemons.went_away(&found) {
                    return Answer::Unanswered(problem);
                }
            }
        }
    }
    Answer::Unanswered(unreachable(
        format!("The Daemon went away {ATTEMPTS} times while this command connected."),
        "Run the command again; `daemon status` shows whether a Daemon runs.",
    ))
}

/// The failure of a command that reached no Daemon, as `message` says,
/// and what to do, `fix`: `daemon_unreachable`, which exits its group's
/// row, `8`.
pub fn unreachable(message: String, fix: &str) -> ErrorObject {
    let details = serde_json::to_value(DaemonUnreachableDetails {
        fix: fix.to_owned(),
    });
    ErrorObject {
        code: DAEMON_UNREACHABLE.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: DAEMON_UNREACHABLE.name.to_owned(),
            retryable: DAEMON_UNREACHABLE.retryable,
            details: match details {
                Ok(Value::Object(details)) => details,
                _ => Map::new(),
            },
        },
    }
}

/// Calls `call` on an Attached engine in this process, `engine` on
/// `dependencies`, opened by a Client that needs `needs`.
///
/// # Panics
///
/// If Tokio cannot build its runtime.
pub fn call_attached(
    engine: &Engine,
    dependencies: Dependencies,
    needs: Product,
    call: CommandCall<'_>,
) -> Answer {
    let runtime = runtime();
    let hello = hello(engine, needs);
    let (network, dialer) = transport::in_memory();
    runtime.block_on(async {
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
                format!("The Attached engine got no Connection: {error}."),
            )),
            (_, answer) => answer,
        }
    })
}

/// The runtime a command's Connection runs on, with the signals an
/// interrupt arrives by.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its threads")
}

/// The `hello` of a command of `engine`'s executable that needs `needs`,
/// with no token.
fn hello(engine: &Engine, needs: Product) -> HelloParams {
    HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Cli,
        form: None,
        token: None,
        needs: Some(needs),
        product_version: engine.identity().product_version.clone(),
        log_level: None,
    }
}

/// The failure of a call whose Connection to the Daemon at `endpoint`
/// dropped before `method` was answered: `connection_lost`, which exits
/// its group's row, `8`.
pub fn connection_lost(endpoint: &DaemonEndpoint, method: &str) -> ErrorObject {
    let mut details = Map::new();
    details.insert(
        "fix".to_owned(),
        Value::from(
            "Check whether the call took effect, then run the command again; `daemon status` shows whether the Daemon still runs.",
        ),
    );
    ErrorObject {
        code: ErrorGroup::Unavailable.json_rpc_class(),
        message: format!(
            "The Connection to the Daemon at {endpoint} dropped before it answered `{method}`, so whether the call took effect is unknown."
        ),
        data: ErrorData {
            code: CONNECTION_LOST.name.to_owned(),
            retryable: CONNECTION_LOST.retryable,
            details,
        },
    }
}

/// Opens `connection` with `hello`, then makes the one call, and closes
/// the Connection once it is answered, which stops the Attached engine.
async fn exchange(connection: Connection, hello: &HelloParams, call: CommandCall<'_>) -> Answer {
    let mut link = Link::of(connection);
    let answer = match link.hello(hello).await {
        Ok(()) => link.call(call).await,
        Err(Opening::Interrupted(interrupt)) => Answer::Interrupted(interrupt),
        Err(Opening::Refused(error) | Opening::HandOver(error)) => Answer::Error(error),
        Err(Opening::Gone | Opening::Unreachable) => closed(HELLO),
    };
    let _ = link.writer.shutdown().await;
    // The call was made: an Attached engine that ended it unanswered fails
    // it, as a bug does.
    match answer {
        Answer::Unanswered(error) => Answer::Error(error),
        answer => answer,
    }
}

/// Why opening a Connection with `hello` failed.
enum Opening {
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

/// The Client's end of the Connection.
struct Link {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    /// The operation of the call in flight, or last made.
    method: String,
}

impl Link {
    fn of(connection: Connection) -> Self {
        let (reader, writer) = connection.split();
        Self {
            lines: BufReader::new(reader).lines(),
            writer,
            method: HELLO.to_owned(),
        }
    }

    /// A Connection to the Daemon at `endpoint`, opened with `hello`.
    async fn open(endpoint: &DaemonEndpoint, hello: &HelloParams) -> Result<Self, Opening> {
        let connection = transport::connect_daemon(endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut link = Self::of(connection);
        match link.hello(hello).await {
            Ok(()) => Ok(link),
            Err(failure) => {
                let _ = link.writer.shutdown().await;
                Err(failure)
            }
        }
    }

    /// Says `hello` and reads its answer.
    async fn hello(&mut self, hello: &HelloParams) -> Result<(), Opening> {
        let interrupt = interrupted();
        tokio::pin!(interrupt);
        let hello = serde_json::to_value(hello).expect("invariant: hello encodes");
        if !self.send(&request(HELLO_ID, HELLO, hello)).await {
            return Err(Opening::Gone);
        }
        loop {
            tokio::select! {
                interrupt = &mut interrupt => return Err(Opening::Interrupted(interrupt)),
                message = self.next() => match message {
                    None => return Err(Opening::Gone),
                    Some(Message::Response(response)) if response.id == RequestId::Number(HELLO_ID) => return Ok(()),
                    Some(Message::Error(failure)) if failure.id == Some(RequestId::Number(HELLO_ID)) => {
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
    /// command then ends with whatever the engine answers.
    #[expect(
        clippy::too_many_lines,
        reason = "one loop reads every message of the command's calls; splitting it would pass its state between functions"
    )]
    async fn call(&mut self, mut call: CommandCall<'_>) -> Answer {
        let interrupt = interrupted();
        tokio::pin!(interrupt);
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
        loop {
            tokio::select! {
                interrupt_now = &mut interrupt => {
                    if cancelled.is_some() || stopping.is_some() {
                        return Answer::Interrupted(interrupt_now);
                    }
                    if let Some(stop) = call.stopper.as_mut().and_then(|stopper| stopper()) {
                        let stop_id = id + 1;
                        if !self.send(&request(stop_id, stop.method, stop.params)).await {
                            return Answer::Interrupted(interrupt_now);
                        }
                        stopping = Some((stop_id, interrupt_now));
                        interrupt.set(interrupted());
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
                    interrupt.set(interrupted());
                }
                message = self.next() => match message {
                    None => return closed(method),
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
                        if let Some(input) = call.input.as_mut()
                            && sent.method == CREDIT
                        {
                            let credits = params["credits"].as_u64().unwrap_or(0);
                            let (values, end) = (input.next)(u32::try_from(credits).unwrap_or(u32::MAX));
                            let mut batch = Map::new();
                            batch.insert("id".to_owned(), Value::from(id));
                            batch.insert(input.field.to_owned(), Value::Array(values));
                            batch.insert("end".to_owned(), Value::Bool(end));
                            if !self.send(&notification(input.method, &batch)).await {
                                return closed(method);
                            }
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

    /// The next message the engine sends, or `None` once it closed the
    /// Connection; a line that is no message is passed over.
    async fn next(&mut self) -> Option<Message> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if let Ok(message) = serde_json::from_str(&line) {
                record_wire(RECEIVED, line.as_bytes());
                return Some(message);
            }
        }
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

/// Resolves when the command is interrupted (`SIGINT`, or Ctrl-C and
/// `CTRL_BREAK_EVENT` on Windows) or sent `SIGTERM`; never for a signal the
/// CLI cannot listen for.
async fn interrupted() -> Interrupt {
    tokio::select! {
        () = signalled(tokio::signal::ctrl_c()) => Interrupt::Interrupted,
        interrupt = other_signals() => interrupt,
    }
}

/// `SIGTERM`, the other signal that ends a command on Unix.
#[cfg(unix)]
async fn other_signals() -> Interrupt {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            terminate.recv().await;
            Interrupt::Terminated
        }
        Err(_) => std::future::pending().await,
    }
}

/// `CTRL_BREAK_EVENT`, which a process group gets where Unix sends
/// `SIGINT` (ADR 0093).
#[cfg(windows)]
async fn other_signals() -> Interrupt {
    match tokio::signal::windows::ctrl_break() {
        Ok(mut ctrl_break) => {
            ctrl_break.recv().await;
            Interrupt::Interrupted
        }
        Err(_) => std::future::pending().await,
    }
}

/// Resolves once `signal` arrives; never when it cannot be listened for.
async fn signalled(signal: impl Future<Output = std::io::Result<()>>) {
    if signal.await.is_err() {
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
