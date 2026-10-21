//! Where a command's call runs (ADR 0024): today an Attached engine in the
//! CLI's own process, which it reaches over the in-memory transport pair,
//! opens with `hello` as the form `cli` (ADRs 0095, 0099), serves this one
//! command's call and stops with it, stopping the Runs it started. The
//! Daemon joins here with the choice between the two.
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI builds its runtime before and outside any async worker, as main does (ADR 0060)"
)]

use goodfetch_engine::transport::{self, Connection, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HelloParams, Surface};
use goodfetch_protocol::jsonrpc::{ErrorObject, Message, Request, RequestId, Version};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

/// How a call ended.
pub enum Answer {
    /// Its result.
    Result(Value),
    /// The engine's error, or its refusal of `hello`.
    Error(ErrorObject),
    /// No answer: the engine could not be reached, or the Connection ended
    /// first.
    Unanswered(String),
}

/// Calls `method` with `params` on an Attached engine in this process,
/// `engine` on `dependencies`, opened by a Client that needs `needs`.
///
/// # Panics
///
/// If Tokio cannot build its runtime.
pub fn call_attached(
    engine: &Engine,
    dependencies: Dependencies,
    needs: Product,
    method: &str,
    params: Value,
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
    };
    let (network, dialer) = transport::in_memory();
    runtime.block_on(async {
        let client = async {
            match dialer.connect().await {
                Ok(connection) => exchange(connection, &hello, method, params).await,
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

/// Opens `connection` with `hello`, then sends the one call, and closes
/// the Connection once it is answered, which stops the Attached engine.
async fn exchange(
    connection: Connection,
    hello: &HelloParams,
    method: &str,
    params: Value,
) -> Answer {
    let (reader, mut writer) = connection.split();
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let hello = serde_json::to_value(hello).expect("invariant: hello encodes");
    let calls = [(0, "hello", hello), (1, method, params)];
    let mut answer = Answer::Unanswered("no call was sent".to_owned());
    for (id, method, params) in calls {
        let request = Request {
            jsonrpc: Version,
            id: RequestId::Number(id),
            method: method.to_owned(),
            params: Some(params),
        };
        let mut line = serde_json::to_vec(&request).expect("invariant: a request encodes");
        line.push(b'\n');
        if writer.write_all(&line).await.is_err() || writer.flush().await.is_err() {
            return Answer::Unanswered(format!(
                "the engine closed the Connection before `{method}`"
            ));
        }
        answer = loop {
            let Ok(Some(line)) = lines.next_line().await else {
                break Answer::Unanswered(format!(
                    "the engine closed the Connection before it answered `{method}`"
                ));
            };
            match serde_json::from_str::<Message>(&line) {
                Ok(Message::Response(response)) if response.id == RequestId::Number(id) => {
                    break Answer::Result(response.result);
                }
                Ok(Message::Error(failure)) if failure.id == Some(RequestId::Number(id)) => {
                    break Answer::Error(failure.error);
                }
                // Events and notifications are no answer to this call.
                _ => {}
            }
        };
        if !matches!(answer, Answer::Result(_)) {
            break;
        }
    }
    let _ = writer.shutdown().await;
    answer
}
