//! Where a command's call runs (ADR 0024): today an Attached engine in the
//! CLI's own process, which it reaches over the in-memory transport pair,
//! opens with `hello` as the form `cli` (ADRs 0095, 0099), serves this one
//! command's call and stops with it, stopping the Runs it started. The
//! Daemon joins here with the choice between the two.
//!
//! A followed call is granted credits as what it sends is printed (ADR
//! 0024). An interrupt (`SIGINT`, Ctrl-C) sends a cancellable call's
//! `$/cancel` and waits for the engine to stop its work; a second
//! interrupt, or one during a call that cannot be cancelled, ends the
//! command at once (ADRs 0069 and 0080).
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI builds its runtime before and outside any async worker, as main does (ADR 0060)"
)]

use goodfetch_engine::transport::{self, Connection, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{CANCEL, CancelParams, HELLO, HelloParams, Surface};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::jsonrpc::{
    ErrorObject, Message, Notification, Request, RequestId, Version,
};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

/// The id of `hello`.
const HELLO_ID: i64 = 0;

/// The id of the command's call.
const CALL_ID: i64 = 1;

/// How many events or chunks a followed call may send beyond those printed.
const CREDITS: u32 = 64;

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
    /// The command was interrupted, and its call cancelled when it could
    /// be.
    Interrupted,
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its threads");
    let hello = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Cli,
        token: None,
        needs: Some(needs),
        product_version: engine.identity().product_version.clone(),
        log_level: None,
    };
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

/// Opens `connection` with `hello`, then makes the one call, and closes
/// the Connection once it is answered, which stops the Attached engine.
async fn exchange(connection: Connection, hello: &HelloParams, call: CommandCall<'_>) -> Answer {
    let (reader, writer) = connection.split();
    let mut link = Link {
        lines: BufReader::new(reader).lines(),
        writer,
    };
    let answer = link.converse(hello, call).await;
    let _ = link.writer.shutdown().await;
    answer
}

/// The Client's end of the Connection.
struct Link {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl Link {
    async fn converse(&mut self, hello: &HelloParams, mut call: CommandCall<'_>) -> Answer {
        let interrupt = interrupted();
        tokio::pin!(interrupt);
        let hello = serde_json::to_value(hello).expect("invariant: hello encodes");
        if !self.send(&request(HELLO_ID, HELLO, hello)).await {
            return Answer::Unanswered(
                "the engine closed the Connection before `hello`".to_owned(),
            );
        }
        loop {
            tokio::select! {
                () = &mut interrupt => return Answer::Interrupted,
                message = self.next() => match message {
                    None => return closed(HELLO),
                    Some(Message::Response(response)) if response.id == RequestId::Number(HELLO_ID) => break,
                    Some(Message::Error(failure)) if failure.id == Some(RequestId::Number(HELLO_ID)) => {
                        return Answer::Error(failure.error);
                    }
                    Some(_) => {}
                },
            }
        }
        let params = std::mem::take(&mut call.params);
        if !self.send(&request(CALL_ID, call.method, params)).await {
            return Answer::Unanswered(format!(
                "the engine closed the Connection before `{}`",
                call.method
            ));
        }
        if call.follower.is_some() && !self.credit(CREDITS).await {
            return closed(call.method);
        }
        let mut cancelled = false;
        loop {
            tokio::select! {
                () = &mut interrupt => {
                    if !call.cancellable || cancelled {
                        return Answer::Interrupted;
                    }
                    let cancel = notification(CANCEL, &CancelParams { id: RequestId::Number(CALL_ID) });
                    if !self.send(&cancel).await {
                        return Answer::Interrupted;
                    }
                    cancelled = true;
                    interrupt.set(interrupted());
                }
                message = self.next() => match message {
                    None => return closed(call.method),
                    Some(Message::Response(response)) if response.id == RequestId::Number(CALL_ID) => {
                        return if cancelled { Answer::Interrupted } else { Answer::Result(response.result) };
                    }
                    Some(Message::Error(failure)) if failure.id == Some(RequestId::Number(CALL_ID)) => {
                        return if cancelled { Answer::Interrupted } else { Answer::Error(failure.error) };
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

/// Resolves when the command is interrupted; never, when the CLI cannot
/// listen for interrupts.
async fn interrupted() {
    if tokio::signal::ctrl_c().await.is_err() {
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
