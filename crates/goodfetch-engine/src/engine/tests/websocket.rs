//! An Exposed Daemon's WebSocket, through the engine's public interface
//! over the in-memory network (spec #213's Seam 3; ADR 0049): on the one
//! address it serves newline-delimited JSON-RPC and the protocol over
//! WebSocket, opened by the same `hello` with its token; credits, positions
//! and `page.body` work over it unchanged; and a handshake with a foreign
//! `Host`, or a foreign or missing `Origin`, is refused with `403` and
//! logged as its listener's refusal (ADRs 0017 and 0067).

use std::net::SocketAddr;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader, Join};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::{Error, Message};

use super::daemon::{TOKEN, daemon as served, hello_with, no_handover, options as daemon_options};
use super::events::{body_bytes, credit, engine as streaming_engine};
use super::{Client, engine, records_of};
use crate::daemon::DaemonOptions;
use crate::endpoint::Authorities;
use crate::endpoint::REFUSAL_TARGET;
use crate::events::EventsOptions;
use crate::transport::{ConnectionReader, ConnectionWriter, InMemoryDialer};

/// The address the Exposed Daemon listens at.
const ADDRESS: &str = "127.0.0.1:7900";

fn address() -> SocketAddr {
    ADDRESS.parse().unwrap()
}

/// An Exposed Daemon's options: its listener at [`ADDRESS`] also carries
/// the protocol over WebSocket.
pub(super) fn exposed() -> DaemonOptions {
    DaemonOptions {
        endpoint: DaemonEndpoint::Tcp(address()),
        websocket: Some(Authorities::loopback(address())),
        ..daemon_options(true, &no_handover())
    }
}

fn token_hello() -> Value {
    hello_with("collie", "0.1", "sdk_typescript", Some(TOKEN))
}

/// A Client's end of a WebSocket.
struct WebSocketClient(WebSocketStream<Join<ConnectionReader, ConnectionWriter>>);

impl WebSocketClient {
    async fn send(&mut self, messages: &[Value]) {
        for message in messages {
            self.0
                .send(Message::text(message.to_string()))
                .await
                .unwrap();
        }
    }

    /// The next message, or `None` once the engine closed the WebSocket.
    async fn next(&mut self) -> Option<Value> {
        loop {
            match self.0.next().await? {
                Ok(Message::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    /// The reply to the call `id`, read past every other message.
    async fn reply(&mut self, id: i64) -> Value {
        loop {
            let message = self
                .next()
                .await
                .expect("the engine closed before replying");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }
}

/// A WebSocket handshake naming `host`, and `origin` when given.
fn handshake(host: &str, origin: Option<&str>) -> Request<()> {
    let mut request = Request::builder()
        .uri(format!("ws://{host}/"))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key());
    if let Some(origin) = origin {
        request = request.header("Origin", origin);
    }
    request.body(()).unwrap()
}

/// Opens a WebSocket to the Daemon with `request`: the Client, or the HTTP
/// status that refused it.
async fn websocket(dialer: &InMemoryDialer, request: Request<()>) -> Result<WebSocketClient, u16> {
    let (reader, writer) = dialer.connect().await.unwrap().split();
    match tokio_tungstenite::client_async(request, tokio::io::join(reader, writer)).await {
        Ok((socket, _)) => Ok(WebSocketClient(socket)),
        Err(Error::Http(response)) => Err(response.status().as_u16()),
        Err(error) => panic!("the handshake failed: {error}"),
    }
}

/// A WebSocket the Daemon's own origin opens.
async fn same_origin(dialer: &InMemoryDialer) -> WebSocketClient {
    websocket(
        dialer,
        handshake(ADDRESS, Some(&format!("http://{ADDRESS}"))),
    )
    .await
    .unwrap_or_else(|status| panic!("the handshake was refused with {status}"))
}

/// Covers: story:3-13
#[test]
fn an_exposed_daemon_serves_the_protocol_over_websocket_and_lines_on_one_address() {
    let (_, (over_websocket, over_lines)) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        exposed(),
        |dialer| async move {
            let mut websocket = same_origin(&dialer).await;
            websocket.send(&[token_hello()]).await;
            let over_websocket = websocket.next().await.unwrap();
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut lines = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            lines.send(&[token_hello()]).await;
            let over_lines = lines.next().await.unwrap();
            (over_websocket, over_lines)
        },
    );

    assert_eq!(
        (
            over_websocket["result"]["product"].clone(),
            over_lines["result"]["product"].clone()
        ),
        (json!("collie"), json!("collie")),
        "{over_websocket} {over_lines}"
    );
}

/// Covers: story:3-13, story:3-11
#[test]
fn a_websocket_that_opens_without_the_token_is_closed_unanswered() {
    let (_, answer) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        exposed(),
        |dialer| async move {
            let mut websocket = same_origin(&dialer).await;
            websocket
                .send(&[hello_with("collie", "0.1", "sdk_typescript", None)])
                .await;
            websocket.next().await
        },
    );

    assert_eq!(answer, None);
}

/// Covers: story:3-13, operation:events.subscribe, operation:page.body, event:run.axes_changed
#[test]
fn credits_positions_and_page_bodies_work_over_websocket_unchanged() {
    const SIZE: usize = 150 * 1024;
    let env = TestEnv::new();
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });

    let (_, (held, events, offsets, bytes, read)) = served(
        &streaming_engine(EventsOptions::default()),
        &env,
        exposed(),
        |dialer| async move {
            let mut websocket = same_origin(&dialer).await;
            websocket.send(&[token_hello()]).await;
            let _ = websocket.next().await;
            websocket
                .send(&[
                    call(1, "events.subscribe", json!({})),
                    call(2, "test.body", json!({ "run_id": "bodies", "size": SIZE })),
                ])
                .await;
            let handle = websocket.reply(2).await["result"].clone();
            // No credit yet: no event comes.
            let held = tokio::time::timeout(std::time::Duration::from_secs(60), websocket.next())
                .await
                .ok()
                .flatten();
            websocket.send(&[credit(1, 100)]).await;
            let mut events = Vec::new();
            while events.is_empty() {
                let message = websocket.next().await.expect("the engine closed");
                if message["method"] == "$/events" && message["params"]["id"] == json!(1) {
                    events.extend(message["params"]["events"].as_array().unwrap().clone());
                }
            }
            websocket
                .send(&[call(
                    3,
                    "page.body",
                    json!({ "run_id": "bodies", "body": handle["body"] }),
                )])
                .await;
            let (mut offsets, mut bytes) = (Vec::new(), Vec::new());
            loop {
                websocket.send(&[credit(3, 1)]).await;
                let message = loop {
                    let message = websocket.next().await.expect("the engine closed");
                    if message["method"] == "$/chunk" || message["id"] == json!(3) {
                        break message;
                    }
                };
                if message["method"] != "$/chunk" {
                    return (held, events, offsets, bytes, message["result"].clone());
                }
                offsets.push(message["params"]["offset"].as_u64().unwrap());
                bytes.extend(
                    data_encoding::BASE64
                        .decode(message["params"]["data"].as_str().unwrap().as_bytes())
                        .unwrap(),
                );
            }
        },
    );

    assert_eq!(
        (
            held,
            events[0]["event"].clone(),
            events[0]["data"]["run_id"].clone(),
            events[0]["position"]
                .as_u64()
                .is_some_and(|position| position >= 1),
            offsets,
            bytes == body_bytes(SIZE),
            read,
        ),
        (
            None,
            json!("run.axes_changed"),
            json!("bodies"),
            true,
            vec![0, 64 * 1024, 128 * 1024],
            true,
            json!({ "size": SIZE }),
        ),
        "{events:#?}"
    );
}

/// Covers: story:3-13
#[rstest]
#[case::host_foreign("foreign.invalid", Some("http://127.0.0.1:7900"), "host_foreign")]
#[case::origin_foreign("127.0.0.1:7900", Some("https://foreign.invalid"), "origin_foreign")]
#[case::origin_of_another_port("127.0.0.1:7900", Some("http://127.0.0.1:7901"), "origin_foreign")]
#[case::origin_missing("127.0.0.1:7900", None, "origin_missing")]
fn a_handshake_with_a_foreign_host_or_a_foreign_or_missing_origin_is_refused_with_403(
    #[case] host: &str,
    #[case] origin: Option<&str>,
    #[case] reason: &str,
) {
    let mut refused = None;
    let records = records_of(REFUSAL_TARGET, || {
        let (_, status) = served(
            &engine(Product::Collie),
            &TestEnv::new(),
            exposed(),
            |dialer| async move { websocket(&dialer, handshake(host, origin)).await.err() },
        );
        refused = status;
    });

    assert_eq!(
        (
            refused,
            records
                .iter()
                .filter(|record| record.contains(&format!(
                    "listener=\"daemon_websocket\" reason=\"{reason}\""
                )))
                .count()
        ),
        (Some(403), 1),
        "{records:#?}"
    );
}

/// Covers: story:3-13
#[test]
fn a_handshake_that_names_the_daemon_as_localhost_is_accepted() {
    let (_, opened) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        exposed(),
        |dialer| async move {
            websocket(
                &dialer,
                handshake("localhost:7900", Some("http://localhost:7900")),
            )
            .await
            .is_ok()
        },
    );

    assert!(opened);
}

/// Covers: story:3-13
#[test]
fn a_handshake_that_offers_the_protocols_major_as_subprotocol_is_answered_with_it() {
    let subprotocol = PROTOCOL_VERSION.websocket_subprotocol();
    let offered = format!("goodfetch.v9999, {subprotocol}");
    let (_, answered) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        exposed(),
        |dialer| async move {
            let mut request = handshake(ADDRESS, Some(&format!("http://{ADDRESS}")));
            request
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", offered.parse().unwrap());
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let (_, response) =
                tokio_tungstenite::client_async(request, tokio::io::join(reader, writer))
                    .await
                    .unwrap();
            response
                .headers()
                .get("Sec-WebSocket-Protocol")
                .map(|value| value.to_str().unwrap().to_owned())
        },
    );

    assert_eq!(answered, Some(subprotocol));
}

/// Covers: story:3-13
#[test]
fn a_message_longer_than_the_limit_is_answered_message_too_large_and_the_websocket_closes() {
    let (_, (answer, after)) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        exposed(),
        |dialer| async move {
            let mut websocket = same_origin(&dialer).await;
            websocket.send(&[token_hello()]).await;
            let _ = websocket.next().await;
            let (mut sink, mut stream) = websocket.0.split();
            let oversized = "x".repeat(crate::rpc::framing::MAX_MESSAGE_BYTES + 1);
            let read = async {
                let mut messages = Vec::new();
                while let Some(Ok(message)) = stream.next().await {
                    if let Message::Text(text) = message {
                        messages.push(serde_json::from_str::<Value>(&text).unwrap());
                    }
                }
                messages
            };
            let (_, mut messages) = tokio::join!(sink.send(Message::text(oversized)), read);
            let answer = messages.remove(0);
            (answer, messages)
        },
    );

    assert_eq!(
        (answer["error"]["data"]["code"].clone(), after),
        (json!("message_too_large"), Vec::<Value>::new()),
        "{answer}"
    );
}
