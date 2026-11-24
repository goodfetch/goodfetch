//! An Exposed Daemon behind TLS terminated upstream, through Collie's
//! executable (ADR 0049): with `daemon.tls.upstream`, `daemon run` still
//! requires its token and holds a WebSocket's `Host` and `Origin` to its
//! own names, reads `X-Forwarded-Host` and `X-Forwarded-Proto` only from a
//! peer `daemon.trusted_proxies` lists, and records the choice in the Audit
//! log as `tls_terminated_upstream`.
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own WebSocket to the Daemon, outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test waits for the Daemon's Audit log from its own thread"
)]

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_support::{DaemonEndpoint, MESSAGE_DEADLINE, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};
use tungstenite::handshake::client::generate_key;
use tungstenite::{HandshakeError, Message, WebSocket};

use crate::daemon_run::{ANY_PORT, Foreground, token};
use crate::endpoint::audit_entries;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The public name an ingress serves the Daemon at.
const PUBLIC: &str = "collie.example.com";

/// A `daemon run` behind TLS terminated upstream, trusting `proxies`, once
/// it serves; and its address.
fn behind_upstream(env: &TestEnv, proxies: &str) -> (Foreground, SocketAddr) {
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.tls.upstream = true\ndaemon.trusted_proxies = {proxies}\n"
    ));
    let mut daemon = Foreground::start(env, COLLIE, &[]);
    let DaemonEndpoint::Tcp(address) = daemon.serving() else {
        panic!("`daemon run` listens on TCP");
    };
    (daemon, address)
}

/// The Daemon at `address`'s answer to a WebSocket handshake naming
/// `host`, from a page of `origin`, with the `forwarded` headers: the
/// WebSocket, or the status it was refused with.
fn handshake(
    address: SocketAddr,
    host: &str,
    origin: &str,
    forwarded: &[(&str, &str)],
) -> Result<WebSocket<TcpStream>, u16> {
    let mut request = tungstenite::http::Request::builder()
        .uri(format!("ws://{address}/"))
        .header("Host", host)
        .header("Origin", origin)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key());
    for (name, value) in forwarded {
        request = request.header(*name, *value);
    }
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    match tungstenite::client(request.body(()).unwrap(), stream) {
        Ok((socket, _)) => Ok(socket),
        Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            Err(response.status().as_u16())
        }
        Err(error) => panic!("the handshake neither opened nor was refused: {error}"),
    }
}

/// The Daemon's reply to a `hello` with `token`, if any, over `socket`;
/// `None` when it closes the WebSocket unanswered, as it does a refused
/// `hello`.
fn hello(socket: &mut WebSocket<TcpStream>, token: Option<&str>) -> Option<Value> {
    let mut params = goodfetch_test_client::Hello::needing(Product::Collie)
        .surface(Surface::SdkTypescript)
        .params();
    if let Some(token) = token {
        params["token"] = json!(token);
    }
    let call = json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": params });
    socket.send(Message::text(call.to_string())).unwrap();
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let message: Value = serde_json::from_str(&text).unwrap();
                if message.get("id").is_some() {
                    let _ = socket.close(None);
                    return Some(message);
                }
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

/// The reasons of the Audit log's refusals of the Daemon's WebSocket, in
/// order, once it holds `count` of them or by `MESSAGE_DEADLINE`.
fn refusals(env: &TestEnv, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + MESSAGE_DEADLINE;
    loop {
        let reasons: Vec<Value> = audit_entries(env)
            .into_iter()
            .filter(|entry| {
                entry["kind"] == json!("endpoint_authentication_failed")
                    && entry["details"]["listener"] == json!("daemon_websocket")
            })
            .map(|entry| entry["details"]["reason"].clone())
            .collect();
        if reasons.len() >= count || Instant::now() > deadline {
            return reasons;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Behind TLS terminated upstream, the Daemon still refuses a `hello`
/// with no token, a handshake naming a foreign `Host` and one from a page
/// of a foreign `Origin`, each in the Audit log, while its own `Host` and
/// `Origin` with its token open; and the Audit log records the choice with
/// the address and the proxies it trusts.
///
/// Covers: story:4-25, story:4-26, setting:daemon.tls.upstream, setting:daemon.trusted_proxies, command:daemon.run
#[test]
fn behind_tls_terminated_upstream_the_daemon_still_requires_its_token_host_and_origin() {
    let env = TestEnv::new();
    let (mut daemon, address) = behind_upstream(&env, "[\"192.0.2.1\"]");
    let own = address.to_string();
    let own_origin = format!("http://{own}");
    let token = token(&env);

    let unsigned = hello(
        &mut handshake(address, &own, &own_origin, &[]).unwrap(),
        None,
    );
    let foreign_host = handshake(address, PUBLIC, &own_origin, &[]).err();
    let foreign_origin = handshake(address, &own, &format!("https://{PUBLIC}"), &[]).err();
    let signed = hello(
        &mut handshake(address, &own, &own_origin, &[]).unwrap(),
        Some(&token),
    );
    let refused = refusals(&env, 3);
    let exited = daemon.terminate();
    let recorded: Vec<Value> = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == json!("tls_terminated_upstream"))
        .map(|entry| entry["details"].clone())
        .collect();

    assert_eq!(
        (
            unsigned,
            foreign_host,
            foreign_origin,
            signed.map(|reply| reply["result"]["product"].clone()),
            refused,
            recorded,
            exited,
        ),
        (
            None,
            Some(403),
            Some(403),
            Some(json!("collie")),
            vec![
                json!("token_missing"),
                json!("host_foreign"),
                json!("origin_foreign"),
            ],
            vec![json!({ "address": own, "trusted_proxies": ["192.0.2.1/32"] })],
            true,
        )
    );
}

/// A handshake an ingress forwarded for [`PUBLIC`] over `https` opens only
/// when its peer, the test on loopback, is in `daemon.trusted_proxies`;
/// from any other peer its `X-Forwarded-*` headers are not read, and its
/// `Host` is foreign.
///
/// Covers: story:4-26, setting:daemon.tls.upstream, setting:daemon.trusted_proxies
#[rstest]
#[case::from_an_unlisted_address("[\"192.0.2.1\"]", Err(403))]
#[case::from_a_listed_address("[\"127.0.0.0/8\"]", Ok(Some(json!("collie"))))]
fn x_forwarded_is_read_only_from_a_trusted_proxy(
    #[case] proxies: &str,
    #[case] expected: Result<Option<Value>, u16>,
) {
    let env = TestEnv::new();
    let (mut daemon, address) = behind_upstream(&env, proxies);
    let token = token(&env);

    let opened = handshake(
        address,
        PUBLIC,
        &format!("https://{PUBLIC}"),
        &[("X-Forwarded-Host", PUBLIC), ("X-Forwarded-Proto", "https")],
    )
    .map(|mut socket| {
        hello(&mut socket, Some(&token)).map(|reply| reply["result"]["product"].clone())
    });
    let exited = daemon.terminate();

    assert_eq!((opened, exited), (expected, true));
}
