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
//! names this executable's `daemon restart`; otherwise the command warns of
//! the refusal and runs on an Attached engine, as `engine: "auto"` does
//! (ADR 0098). A Daemon that hands over is waited for. A Connection to the
//! Daemon that drops before the call is answered fails it with
//! `connection_lost`. The token the Daemon takes never leaves its `hello`.
//!
//! A followed call is granted credits as what it sends is printed (ADR
//! 0024). An interrupt (`SIGINT`, Ctrl-C, or `CTRL_BREAK_EVENT` on Windows)
//! or `SIGTERM` sends a cancellable call's `$/cancel` and waits for the
//! engine to stop its work; a second one, or one during a call that cannot
//! be cancelled, ends the command at once (ADRs 0069, 0080 and 0093).
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
use goodfetch_protocol::daemon::DaemonEndpoint;
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
    pub token: String,
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
    /// What kept it from finding or starting one, for people.
    fn find(&self, start: bool) -> Result<Option<Reached>, String>;

    /// Waits until a Daemon that refused the command and hands over has
    /// started its major's pick.
    ///
    /// # Errors
    ///
    /// What kept it from waiting, for people.
    fn handed_over(&self) -> Result<(), String>;

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
/// params, and answers how many credits printing it used.
pub type Follower<'a> = &'a mut dyn FnMut(&str, &Value) -> u32;

/// The one call a command makes.
pub struct CommandCall<'a> {
    /// The operation.
    pub method: &'a str,
    /// Its params.
    pub params: Value,
    /// Whether `$/cancel` stops it.
    pub cancellable: bool,
    /// For a sequence run with `--follow`, what prints what it sends.
    pub follower: Option<Follower<'a>>,
}

/// How a call ended.
pub enum Answer {
    /// Its result.
    Result(Value),
    /// The engine's error, or its refusal of `hello`.
    Error(ErrorObject),
    /// No answer: the engine could not be reached, or the Connection ended
    /// first.
    Unanswered(String),
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
            token: Some(found.token.clone()),
            ..hello(engine, needs)
        };
        match runtime.block_on(Link::open(&found.endpoint, &hello)) {
            Ok(mut link) => {
                let method = call.method.to_owned();
                let answer = runtime.block_on(async {
                    let answer = link.call(call).await;
                    let _ = link.writer.shutdown().await;
                    answer
                });
                return match answer {
                    Answer::Unanswered(_) => {
                        Answer::Error(connection_lost(&found.endpoint, &method))
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
                if reach.choice == Choice::Daemon {
                    return Answer::Error(refusal);
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
                return Answer::Unanswered(format!(
                    "the Daemon at {} closed the Connection before it answered `hello`: check that `GOODFETCH_TOKEN` holds its token",
                    found.endpoint
                ));
            }
            Err(Opening::Unreachable) if found.remote => {
                return Answer::Unanswered(format!(
                    "nothing answers at {}, the Daemon `GOODFETCH_URL` names",
                    found.endpoint
                ));
            }
            // It went away in between: look again.
            Err(Opening::Gone | Opening::Unreachable) => {}
        }
    }
    Answer::Unanswered(format!(
        "the Daemon went away {ATTEMPTS} times while this command connected"
    ))
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
                Err(error) => Answer::Unanswered(format!("the engine took no Connection: {error}")),
            }
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        match (served, answer) {
            (Err(error), Answer::Unanswered(_)) => {
                Answer::Unanswered(format!("the Attached engine got no Connection: {error}"))
            }
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
/// dropped before `method` was answered.
fn connection_lost(endpoint: &DaemonEndpoint, method: &str) -> ErrorObject {
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
    answer
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
}

impl Link {
    fn of(connection: Connection) -> Self {
        let (reader, writer) = connection.split();
        Self {
            lines: BufReader::new(reader).lines(),
            writer,
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

    /// Makes the one call on the opened Connection and reads its answer.
    async fn call(&mut self, mut call: CommandCall<'_>) -> Answer {
        let interrupt = interrupted();
        tokio::pin!(interrupt);
        let params = std::mem::take(&mut call.params);
        if !self.send(&request(CALL_ID, call.method, params)).await {
            return closed(call.method);
        }
        if call.follower.is_some() && !self.credit(CREDITS).await {
            return closed(call.method);
        }
        let mut cancelled = None;
        loop {
            tokio::select! {
                interrupt_now = &mut interrupt => {
                    if !call.cancellable || cancelled.is_some() {
                        return Answer::Interrupted(interrupt_now);
                    }
                    let cancel = notification(CANCEL, &CancelParams { id: RequestId::Number(CALL_ID) });
                    if !self.send(&cancel).await {
                        return Answer::Interrupted(interrupt_now);
                    }
                    cancelled = Some(interrupt_now);
                    interrupt.set(interrupted());
                }
                message = self.next() => match message {
                    None => return closed(call.method),
                    Some(Message::Response(response)) if response.id == RequestId::Number(CALL_ID) => {
                        return cancelled.map_or(Answer::Result(response.result), Answer::Interrupted);
                    }
                    Some(Message::Error(failure)) if failure.id == Some(RequestId::Number(CALL_ID)) => {
                        return cancelled.map_or(Answer::Error(failure.error), Answer::Interrupted);
                    }
                    Some(Message::Notification(sent)) => {
                        let Some(follower) = call.follower.as_mut() else { continue };
                        let params = sent.params.unwrap_or(Value::Null);
                        if params["id"] != CALL_ID {
                            continue;
                        }
                        let used = follower(&sent.method, &params);
                        if used > 0 && !self.credit(used).await {
                            return closed(call.method);
                        }
                    }
                    Some(_) => {}
                },
            }
        }
    }

    /// Grants the call `credits` more events or chunks.
    async fn credit(&mut self, credits: u32) -> bool {
        let id = RequestId::Number(CALL_ID);
        self.send(&notification(CREDIT, &CreditParams { id, credits }))
            .await
    }

    /// Writes `message` as one line; `false` when the engine is gone.
    async fn send(&mut self, message: &impl Serialize) -> bool {
        let mut line = serde_json::to_vec(message).expect("invariant: a message encodes");
        line.push(b'\n');
        self.writer.write_all(&line).await.is_ok() && self.writer.flush().await.is_ok()
    }

    /// The next message the engine sends, or `None` once it closed the
    /// Connection; a line that is no message is passed over.
    async fn next(&mut self) -> Option<Message> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if let Ok(message) = serde_json::from_str(&line) {
                return Some(message);
            }
        }
    }
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

fn closed(method: &str) -> Answer {
    Answer::Unanswered(format!(
        "the engine closed the Connection before it answered `{method}`"
    ))
}
