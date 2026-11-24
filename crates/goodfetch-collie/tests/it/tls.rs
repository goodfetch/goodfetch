//! An Exposed Daemon's TLS, through Collie's executable (ADR 0049): with
//! `daemon.tls.cert_file` and `daemon.tls.key_file`, `daemon run` serves
//! newline-delimited JSON-RPC and WebSocket over TLS on its address, and a
//! certificate renewed in place from the next Connection, with no restart.
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own TLS Connections to the Daemon, outside the engine (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the Daemon's certificate files, and replaces them while it serves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_test_support::{DaemonEndpoint, TestEnv};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::{Value, json};
use tungstenite::Message;
use tungstenite::handshake::client::generate_key;

use crate::daemon_hello;
use crate::daemon_run::{ANY_PORT, Foreground, token};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The name the Daemon's certificates are for, which its Clients reach it
/// by.
pub(crate) const SERVER_NAME: &str = "localhost";

/// A self-signed certificate for [`SERVER_NAME`]: its file and its key's,
/// PEM, and the certificate as a Client trusts it.
pub(crate) struct Certificate {
    file: String,
    key_file: String,
    pub(crate) trusted: CertificateDer<'static>,
}

impl Certificate {
    pub(crate) fn generated() -> Self {
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = rcgen::CertificateParams::new(vec![SERVER_NAME.to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        Self {
            file: certificate.pem(),
            key_file: key.serialize_pem(),
            trusted: certificate.der().clone(),
        }
    }
}

/// `file` replaced by one holding `contents`, renamed over it as a
/// certificate's renewal replaces it.
fn replace(file: &Path, contents: &str) {
    let staged = file.with_extension("new");
    std::fs::write(&staged, contents).unwrap();
    std::fs::rename(&staged, file).unwrap();
}

/// A `daemon run` on loopback that serves TLS with `served`, once it
/// serves; its address; and its certificate file and key file.
pub(crate) fn serving_tls(
    env: &TestEnv,
    served: &Certificate,
) -> (Foreground, SocketAddr, [PathBuf; 2]) {
    let files = ["daemon.crt", "daemon.key"].map(|name| env.folder().join(name));
    replace(&files[0], &served.file);
    replace(&files[1], &served.key_file);
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.tls.cert_file = \"{}\"\ndaemon.tls.key_file = \"{}\"\n",
        files[0].display(),
        files[1].display(),
    ));
    let mut daemon = Foreground::start(env, COLLIE, &[]);
    let DaemonEndpoint::Tcp(address) = daemon.serving() else {
        panic!("`daemon run` listens on TCP");
    };
    (daemon, address, files)
}

/// A TLS Connection to `address`, naming [`SERVER_NAME`], from a Client
/// that trusts only `trusted`.
fn connected(
    address: SocketAddr,
    trusted: &CertificateDer<'static>,
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(trusted.clone()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connection =
        rustls::ClientConnection::new(Arc::new(config), ServerName::try_from(SERVER_NAME).unwrap())
            .unwrap();
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    rustls::StreamOwned::new(connection, stream)
}

/// The certificate the Daemon at `address` presents to a Client trusting
/// `trusted`, and its reply to that Client's `hello` with `token`, over
/// newline-delimited JSON-RPC.
fn hello_over_lines(
    address: SocketAddr,
    trusted: &CertificateDer<'static>,
    token: &str,
) -> (Option<CertificateDer<'static>>, Value) {
    let mut stream = connected(address, trusted);
    let mut line = daemon_hello(token).message().to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).unwrap();
    let mut reader = BufReader::new(stream);
    let reply = loop {
        let mut answer = String::new();
        reader.read_line(&mut answer).unwrap();
        let message: Value = serde_json::from_str(&answer).unwrap();
        if message.get("id").is_some() {
            break message;
        }
    };
    let presented = reader
        .get_ref()
        .conn
        .peer_certificates()
        .and_then(|chain| chain.first().cloned());
    (presented, reply)
}

/// Whether the Daemon at `address` closes, unanswered, a Connection that
/// opens in plain with `sent`, and the listener and reason of the refusal
/// it logs.
fn refused_in_plain(daemon: &mut Foreground, address: SocketAddr, sent: &str) -> (bool, Value) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    stream.write_all(sent.as_bytes()).unwrap();
    let mut answer = Vec::new();
    let closed =
        std::io::Read::read_to_end(&mut stream, &mut answer).is_ok_and(|_| answer.is_empty());
    let refusal = daemon.refusal().unwrap_or_default();
    (closed, json!([refusal["listener"], refusal["reason"]]))
}

/// The Daemon's reply to a `hello` with `token` over a WebSocket on TLS,
/// whose handshake names the Daemon by [`SERVER_NAME`], from a page of its
/// own `https` origin.
fn hello_over_websocket(
    address: SocketAddr,
    trusted: &CertificateDer<'static>,
    token: &str,
) -> Value {
    let authority = format!("{SERVER_NAME}:{}", address.port());
    let request = tungstenite::http::Request::builder()
        .uri(format!("wss://{authority}/"))
        .header("Host", &authority)
        .header("Origin", format!("https://{authority}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .body(())
        .unwrap();
    let (mut socket, _) = tungstenite::client(request, connected(address, trusted)).unwrap();
    socket
        .send(Message::text(daemon_hello(token).message().to_string()))
        .unwrap();
    loop {
        if let Message::Text(text) = socket.read().unwrap() {
            let message: Value = serde_json::from_str(&text).unwrap();
            if message.get("id").is_some() {
                let _ = socket.close(None);
                return message;
            }
        }
    }
}

/// With a certificate file and its key file, `daemon run` serves `hello`
/// over TLS, on newline-delimited JSON-RPC and on WebSocket alike, and
/// closes unanswered a `hello` or a WebSocket handshake sent in plain, each
/// logged as `tls_required` by its listener; once
/// both files are replaced while it serves, as a renewal replaces them, the
/// next Connection is served the new certificate, with no restart.
///
/// Covers: story:4-25, setting:daemon.tls.cert_file, setting:daemon.tls.key_file, command:daemon.run
#[test]
fn an_exposed_daemon_serves_over_tls_and_a_renewed_certificate_with_no_restart() {
    let env = TestEnv::new();
    let (first, renewed) = (Certificate::generated(), Certificate::generated());
    let (mut daemon, address, [certificate_file, key_file]) = serving_tls(&env, &first);
    let token = token(&env);

    let (before, opened) = hello_over_lines(address, &first.trusted, &token);
    let over_websocket = hello_over_websocket(address, &first.trusted, &token);
    let plain_hello = refused_in_plain(
        &mut daemon,
        address,
        &format!("{}\n", daemon_hello(&token).message()),
    );
    let plain_handshake = refused_in_plain(
        &mut daemon,
        address,
        &format!(
            "GET / HTTP/1.1\r\nHost: {SERVER_NAME}:{port}\r\nOrigin: http://{SERVER_NAME}:{port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {}\r\n\r\n",
            generate_key(),
            port = address.port(),
        ),
    );
    replace(&certificate_file, &renewed.file);
    replace(&key_file, &renewed.key_file);
    let (after, reopened) = hello_over_lines(address, &renewed.trusted, &token);
    let exited = daemon.terminate();

    let product = |reply: &Value| reply["result"]["product"].clone();
    assert_eq!(
        (
            before,
            product(&opened),
            product(&over_websocket),
            plain_hello,
            plain_handshake,
            after,
            product(&reopened),
            exited,
        ),
        (
            Some(first.trusted),
            json!("collie"),
            json!("collie"),
            (true, json!(["exposed_daemon", "tls_required"])),
            (true, json!(["daemon_websocket", "tls_required"])),
            Some(renewed.trusted),
            json!("collie"),
            true,
        ),
        "{:#?}",
        daemon.logged
    );
}
