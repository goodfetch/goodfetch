//! An Exposed Daemon's WebSocket, through Collie's executable (spec #213's
//! Seam 1; ADR 0049): `daemon run` serves the protocol over WebSocket on
//! its configured `daemon.address`, beside newline-delimited JSON-RPC, with
//! the same `hello`, events and credits; and both are columns of the
//! endpoint table, the WebSocket's handshake refusing a foreign `Host` and
//! a foreign or missing `Origin` (ADR 0067), each refusal checked in the
//! Daemon's log until the Audit log records it (area 4).
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own WebSocket to the Daemon, outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{SocketAddr, TcpStream};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_support::{
    AuditUnreadable, DaemonEndpoint, Endpoint, EndpointRunner, Hello, Listener, ListenerAddress,
    ListenerKind, Rule, TestEnv,
};
use serde_json::{Value, json};
use tungstenite::handshake::client::generate_key;
use tungstenite::{Message, WebSocket};

use crate::daemon_run::{ANY_PORT, Foreground, token};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// A `daemon run` listening at a free loopback address, once it serves.
fn exposed(env: &TestEnv) -> (Foreground, SocketAddr) {
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
    let mut daemon = Foreground::start(env, COLLIE, &[]);
    let DaemonEndpoint::Tcp(address) = daemon.serving() else {
        panic!("`daemon run` listens on TCP");
    };
    (daemon, address)
}

/// A `hello`'s params, without its token.
fn hello_params() -> Value {
    goodfetch_test_client::Hello::needing(Product::Collie)
        .surface(Surface::SdkTypescript)
        .params()
}

/// The endpoint table's reason each rule's refusal is logged with.
fn reason(rule: Rule) -> Option<&'static str> {
    Some(match rule {
        Rule::TokenMissing | Rule::TokenInQueryString => "token_missing",
        Rule::TokenWrong | Rule::TokenFromAnotherListener => "token_wrong",
        Rule::HostForeign => "host_foreign",
        Rule::OriginForeign => "origin_foreign",
        Rule::OriginMissing => "origin_missing",
        Rule::HelloNotFirst => "hello_not_first",
        Rule::ProbesAnswerStatusOnly
        | Rule::TokenExpired
        | Rule::TokenRevoked
        | Rule::TokenScopedElsewhere
        | Rule::OriginMissingWithoutHeaderToken => return None,
    })
}

/// The endpoint table's name of each listener `daemon run` serves.
fn listener_name(kind: ListenerKind) -> &'static str {
    match kind {
        ListenerKind::DaemonWebsocket => "daemon_websocket",
        ListenerKind::ExposedDaemon => "exposed_daemon",
        ListenerKind::DaemonSocket => "daemon_socket",
        ListenerKind::PrivateEndpoint => "private_endpoint",
        ListenerKind::OpsListener => "ops_listener",
        ListenerKind::ProxyRelay => "proxy_relay",
    }
}

/// The Daemon's WebSocket and its address, read as the endpoint table's
/// two columns: each passes every row, each refusal logged with its
/// listener and reason.
///
/// Covers: story:3-13, command:daemon.run
#[test]
fn the_daemons_websocket_and_the_exposed_daemon_pass_their_endpoint_table_rows() {
    let env = TestEnv::new();
    let (mut daemon, address) = exposed(&env);
    let token = token(&env);
    let hello = Hello::new(hello_params().as_object().unwrap().clone(), "token");
    let listeners = [
        Listener::new(
            ListenerKind::DaemonWebsocket,
            Endpoint::WebSocket {
                address,
                hello: hello.clone(),
            },
            token.clone(),
        ),
        Listener::new(
            ListenerKind::ExposedDaemon,
            Endpoint::JsonRpc {
                address: ListenerAddress::Tcp(address),
                hello,
            },
            token,
        ),
    ]
    .map(|listener| listener.with_other_listener_token("a-private-endpoints-token"));

    let ran = listeners.map(|listener| {
        let verifier = |kind: ListenerKind, rule: Rule| -> Result<bool, AuditUnreadable> {
            let Some(reason) = reason(rule) else {
                return Ok(false);
            };
            let refusal = daemon.refusal();
            Ok(refusal.is_some_and(|refusal| {
                refusal["listener"] == json!(listener_name(kind))
                    && refusal["reason"] == json!(reason)
            }))
        };
        EndpointRunner::new(verifier).run(&listener)
    });
    let exited = daemon.terminate();

    assert_eq!((ran, exited), ([Ok(()), Ok(())], true));
}

/// A WebSocket Client of `daemon run` at `address`, from the Daemon's own
/// origin.
fn websocket(address: SocketAddr) -> WebSocket<TcpStream> {
    let request = tungstenite::http::Request::builder()
        .uri(format!("ws://{address}/"))
        .header("Host", address.to_string())
        .header("Origin", format!("http://{address}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .body(())
        .unwrap();
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(60)))
        .unwrap();
    tungstenite::client(request, stream).unwrap().0
}

fn send(socket: &mut WebSocket<TcpStream>, message: &Value) {
    socket.send(Message::text(message.to_string())).unwrap();
}

/// The next JSON-RPC message.
fn next(socket: &mut WebSocket<TcpStream>) -> Value {
    loop {
        if let Message::Text(text) = socket.read().unwrap() {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// The reply to the call `id`, and every message read before it.
fn reply(socket: &mut WebSocket<TcpStream>, id: i64) -> (Value, Vec<Value>) {
    let mut before = Vec::new();
    loop {
        let message = next(socket);
        if message["id"] == json!(id) {
            return (message, before);
        }
        before.push(message);
    }
}

fn call(id: i64, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// Over the WebSocket of `daemon run`'s configured address, a Client opens
/// with `hello` and its token, starts a Run, subscribes to the event stream
/// from the start of the Engine instance its `hello` named, grants credits
/// and reads the Run's axis change at its position, as over any other
/// transport.
///
/// Covers: story:3-13, operation:events.subscribe, event:run.axes_changed, operation:run.start
#[test]
fn a_client_calls_daemon_run_over_websocket_with_events_and_credits() {
    let env = TestEnv::new();
    let (mut daemon, address) = exposed(&env);
    let mut socket = websocket(address);
    let mut hello = hello_params();
    hello["token"] = json!(token(&env));

    send(&mut socket, &call(0, "hello", &hello));
    let (opened, _) = reply(&mut socket, 0);
    send(
        &mut socket,
        &call(2, "run.start", &json!({ "run_id": "over-websocket" })),
    );
    let (started, _) = reply(&mut socket, 2);
    let after = json!({
        "after": { "engine_instance": opened["result"]["engine_instance"], "position": 0 },
    });
    send(&mut socket, &call(1, "events.subscribe", &after));
    send(
        &mut socket,
        &json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 1, "credits": 10 } }),
    );
    let mut events = Vec::new();
    while events.is_empty() {
        let message = next(&mut socket);
        if message["method"] == "$/events" && message["params"]["id"] == json!(1) {
            events.extend(message["params"]["events"].as_array().unwrap().clone());
        }
    }
    send(
        &mut socket,
        &call(3, "run.finish", &json!({ "run_id": "over-websocket" })),
    );
    let (finished, _) = reply(&mut socket, 3);
    let _ = socket.close(None);
    let exited = daemon.terminate();

    assert_eq!(
        (
            opened["result"]["product"].clone(),
            started["result"]["run_id"].clone(),
            events[0]["event"].clone(),
            events[0]["data"]["run_id"].clone(),
            events[0]["position"]
                .as_u64()
                .is_some_and(|position| position >= 1),
            finished["result"]["lifecycle"].clone(),
            exited,
        ),
        (
            json!("collie"),
            json!("over-websocket"),
            json!("run.axes_changed"),
            json!("over-websocket"),
            true,
            json!("finished"),
            true,
        ),
        "{events:#?}"
    );
}
