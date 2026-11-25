//! Fixture listeners for the endpoint runner's tests: a JSON-RPC stream
//! listener, a WebSocket listener, each in plain or terminating TLS, an
//! HTTP ops listener, a signed-address listener and an HTTP proxy, each
//! strict (it refuses every row as the table says and records each refusal
//! in its own Audit log, a channel), permissive (it accepts everything) or
//! refusing everything.

use std::io::{BufRead as _, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use goodfetch_test_support::Rule;
use rustls::pki_types::PrivateKeyDer;

/// The token every fixture listener issues.
pub(super) const TOKEN: &str = "fixture-token-7Hq2";

/// How a fixture listener treats what it receives.
#[derive(Clone, Copy, Debug)]
pub(super) enum Behaviour {
    /// Refuses as the endpoint table says, recording each refusal.
    Strict,
    /// Accepts every message and request.
    Permissive,
    /// Refuses everything, the valid `hello` or request included.
    RefuseAll,
}

/// The reason a strict fixture listener records in its Audit log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reason {
    NotHello,
    TokenMissing,
    TokenRefused,
    TokenInQuery,
    ForeignHost,
    ForeignOrigin,
    MissingOrigin,
    PlainWhereTls,
}

/// A fixture listener's Audit log, read as a product's verifier reads its
/// own: each refusal's entry is the next one.
pub(super) struct AuditLog(Receiver<Reason>);

impl AuditLog {
    /// Whether the next entry records `rule`'s refusal.
    pub(super) fn records(&mut self, rule: Rule) -> bool {
        let expected = match rule {
            Rule::TokenMissing | Rule::OriginMissingWithoutHeaderToken => Reason::TokenMissing,
            Rule::TokenWrong
            | Rule::TokenFromAnotherListener
            | Rule::TokenExpired
            | Rule::TokenRevoked
            | Rule::TokenScopedElsewhere => Reason::TokenRefused,
            Rule::TokenInQueryString => Reason::TokenInQuery,
            Rule::HostForeign => Reason::ForeignHost,
            Rule::OriginForeign => Reason::ForeignOrigin,
            Rule::OriginMissing => Reason::MissingOrigin,
            Rule::HelloNotFirst => Reason::NotHello,
            Rule::PlainWhereTlsConfigured => Reason::PlainWhereTls,
            Rule::ProbesAnswerStatusOnly => return false,
        };
        self.0.recv_timeout(Duration::from_secs(5)) == Ok(expected)
    }
}

/// The name a fixture listener that terminates TLS is reached by, which its
/// certificate is for.
pub(super) const SERVER_NAME: &str = "localhost";

/// A fixture listener's TLS: a self-signed certificate for
/// [`SERVER_NAME`], in DER as its Clients trust it, and the server's side.
pub(super) struct FixtureTls {
    pub(super) trusted: Vec<u8>,
    config: Arc<rustls::ServerConfig>,
}

impl FixtureTls {
    fn generated() -> Self {
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = rcgen::CertificateParams::new(vec![SERVER_NAME.to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
        Self {
            trusted: certificate.der().to_vec(),
            config: Arc::new(config),
        }
    }
}

/// A Connection's stream, in plain or over TLS.
trait Stream: Read + Write {}

impl<S: Read + Write> Stream for S {}

/// What a listener that terminates `tls` serves on `stream`: the TLS
/// stream, or a Connection in plain, which only a permissive listener
/// serves and a strict one refuses, recording it.
fn terminated(
    stream: TcpStream,
    tls: &FixtureTls,
    behaviour: Behaviour,
    audit: &Sender<Reason>,
) -> Option<Box<dyn Stream>> {
    let mut first = [0_u8; 1];
    let handshake = stream.peek(&mut first).unwrap_or(0) == 1 && first[0] == 0x16;
    if handshake {
        let connection = rustls::ServerConnection::new(Arc::clone(&tls.config)).unwrap();
        return Some(Box::new(rustls::StreamOwned::new(connection, stream)));
    }
    match behaviour {
        Behaviour::Permissive => Some(Box::new(stream)),
        Behaviour::Strict => {
            audit.send(Reason::PlainWhereTls).unwrap();
            None
        }
        Behaviour::RefuseAll => None,
    }
}

/// Starts a JSON-RPC stream listener on loopback TCP.
pub(super) fn json_rpc_on_tcp(behaviour: Behaviour) -> (SocketAddr, AuditLog) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, log) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            serve_json_rpc(connection.unwrap(), behaviour, &sender);
        }
    });
    (address, AuditLog(log))
}

/// Starts a JSON-RPC stream listener on loopback TCP that terminates TLS,
/// and gives the certificate its Clients trust, in DER.
pub(super) fn json_rpc_over_tls(behaviour: Behaviour) -> (SocketAddr, AuditLog, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, log) = mpsc::channel();
    let tls = FixtureTls::generated();
    let trusted = tls.trusted.clone();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            if let Some(stream) = terminated(connection.unwrap(), &tls, behaviour, &sender) {
                serve_json_rpc(stream, behaviour, &sender);
            }
        }
    });
    (address, AuditLog(log), trusted)
}

/// Starts a JSON-RPC stream listener on a Unix socket at `path`.
#[cfg(unix)]
pub(super) fn json_rpc_on_unix(path: &std::path::Path, behaviour: Behaviour) -> AuditLog {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    let (sender, log) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            serve_json_rpc(connection.unwrap(), behaviour, &sender);
        }
    });
    AuditLog(log)
}

fn serve_json_rpc<S: Read + Write>(stream: S, behaviour: Behaviour, audit: &Sender<Reason>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let refusal = match behaviour {
        Behaviour::Permissive => None,
        Behaviour::RefuseAll => return,
        Behaviour::Strict => first_message_refusal(&line, false),
    };
    if let Some(reason) = refusal {
        audit.send(reason).unwrap();
        return;
    }
    let reply = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
    if reader.get_mut().write_all(reply.as_bytes()).is_err() {
        return;
    }
    let _ = reader.read_to_end(&mut Vec::new());
}

/// The reason a strict listener refuses a first message, or none.
fn first_message_refusal(line: &str, query_token: bool) -> Option<Reason> {
    let message: serde_json::Value = serde_json::from_str(line).unwrap();
    if message["method"] != "hello" {
        return Some(Reason::NotHello);
    }
    match message["params"]["token"].as_str() {
        None if query_token => Some(Reason::TokenInQuery),
        None => Some(Reason::TokenMissing),
        Some(TOKEN) => None,
        Some(_) => Some(Reason::TokenRefused),
    }
}

/// Starts a WebSocket listener on loopback TCP, whose handshake must name
/// its address as `Host` and `http://<address>` as `Origin`.
pub(super) fn websocket(behaviour: Behaviour) -> (SocketAddr, AuditLog) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, log) = mpsc::channel();
    let authority = address.to_string();
    let origin = format!("http://{authority}");
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            serve_websocket(
                connection.unwrap(),
                [&authority, &origin],
                behaviour,
                &sender,
            );
        }
    });
    (address, AuditLog(log))
}

/// Starts a WebSocket listener on loopback TCP that terminates TLS, whose
/// handshake must name [`SERVER_NAME`] and its port as `Host` and
/// `https://` with them as `Origin`, and gives the certificate its Clients
/// trust, in DER.
pub(super) fn websocket_over_tls(behaviour: Behaviour) -> (SocketAddr, AuditLog, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, log) = mpsc::channel();
    let tls = FixtureTls::generated();
    let trusted = tls.trusted.clone();
    let authority = format!("{SERVER_NAME}:{}", address.port());
    let origin = format!("https://{authority}");
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            if let Some(stream) = terminated(connection.unwrap(), &tls, behaviour, &sender) {
                serve_websocket(stream, [&authority, &origin], behaviour, &sender);
            }
        }
    });
    (address, AuditLog(log), trusted)
}

/// Serves one WebSocket on `stream`, whose handshake must name `expected`'s
/// `Host` and `Origin`.
fn serve_websocket<S: Read + Write>(
    stream: S,
    expected: [&str; 2],
    behaviour: Behaviour,
    audit: &Sender<Reason>,
) {
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    let [authority, expected_origin] = expected;
    let mut refused = None;
    let mut query_token = false;
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's handshake callback answers its own `ErrorResponse`"
    )]
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let header = |name: &str| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        query_token = request
            .uri()
            .query()
            .is_some_and(|query| query.contains("access_token"));
        let refusal = match behaviour {
            Behaviour::Permissive => None,
            Behaviour::RefuseAll => Some(None),
            Behaviour::Strict if header("host") != Some(authority) => {
                Some(Some(Reason::ForeignHost))
            }
            Behaviour::Strict => match header("origin") {
                None => Some(Some(Reason::MissingOrigin)),
                Some(origin) if origin != expected_origin => Some(Some(Reason::ForeignOrigin)),
                Some(_) => None,
            },
        };
        match refusal {
            None => Ok(response),
            Some(reason) => {
                refused = Some(reason);
                let mut forbidden = ErrorResponse::new(None);
                *forbidden.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
                Err(forbidden)
            }
        }
    };
    let accepted = tungstenite::accept_hdr(stream, check).ok();
    if let Some(Some(reason)) = refused {
        audit.send(reason).unwrap();
    }
    let Some(mut socket) = accepted else { return };
    let Ok(tungstenite::Message::Text(first)) = socket.read() else {
        return;
    };
    let refusal = match behaviour {
        Behaviour::Strict => first_message_refusal(&first, query_token),
        Behaviour::Permissive | Behaviour::RefuseAll => None,
    };
    if let Some(reason) = refusal {
        audit.send(reason).unwrap();
        let _ = socket.close(None);
        while socket.read().is_ok() {}
        return;
    }
    let reply = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}";
    if socket.send(tungstenite::Message::text(reply)).is_err() {
        return;
    }
    while socket.read().is_ok() {}
}

/// Starts an HTTP ops listener on loopback TCP.
pub(super) fn ops_listener(behaviour: Behaviour) -> (SocketAddr, AuditLog) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, log) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            serve_http(connection.unwrap(), address, behaviour, &sender);
        }
    });
    (address, AuditLog(log))
}

fn serve_http(
    stream: TcpStream,
    address: SocketAddr,
    behaviour: Behaviour,
    audit: &Sender<Reason>,
) {
    let mut reader = BufReader::new(stream);
    let mut target = String::new();
    let mut headers = Vec::new();
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
        if target.is_empty() {
            line.split(' ')
                .nth(1)
                .unwrap_or_default()
                .clone_into(&mut target);
        } else if let Some((name, value)) = line.trim_end().split_once(':') {
            headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        line.clear();
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(found, _)| found == name)
            .map(|(_, value)| value.as_str())
    };
    let authority = address.to_string();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let (status, body) = match behaviour {
        Behaviour::Permissive => (200, "ok"),
        Behaviour::RefuseAll => (403, ""),
        Behaviour::Strict => {
            let refusal = if header("host") != Some(authority.as_str()) {
                Some((403, Reason::ForeignHost))
            } else if header("origin").is_some_and(|origin| origin != format!("http://{authority}"))
            {
                Some((403, Reason::ForeignOrigin))
            } else if path == "/livez" || path == "/readyz" {
                None
            } else if query.contains("access_token") {
                Some((401, Reason::TokenInQuery))
            } else {
                match header("authorization") {
                    None => Some((401, Reason::TokenMissing)),
                    Some(value) if value == format!("Bearer {TOKEN}") => None,
                    Some(_) => Some((401, Reason::TokenRefused)),
                }
            };
            match refusal {
                Some((status, reason)) => {
                    audit.send(reason).unwrap();
                    (status, "")
                }
                None if path == "/metrics" => (200, "goodfetch_up 1\n"),
                None => (200, ""),
            }
        }
    };
    let response = format!(
        "HTTP/1.1 {status} Fixture\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = reader.get_mut().write_all(response.as_bytes());
}

/// The username of a fixture HTTP proxy's credentials, whose password is
/// [`TOKEN`].
pub(super) const PROXY_USERNAME: &str = "goodfetch";

/// Starts an HTTP proxy on loopback TCP, which tunnels only for the
/// credentials of [`PROXY_USERNAME`] and [`TOKEN`] and asks a client
/// without credentials for them.
pub(super) fn http_proxy(behaviour: Behaviour) -> (SocketAddr, AuditLog) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, log) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            serve_proxy(connection.unwrap(), behaviour, &sender);
        }
    });
    (address, AuditLog(log))
}

fn serve_proxy(stream: TcpStream, behaviour: Behaviour, audit: &Sender<Reason>) {
    let mut reader = BufReader::new(stream);
    let mut authorization = None;
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
        if let Some((name, value)) = line.trim_end().split_once(':')
            && name.eq_ignore_ascii_case("proxy-authorization")
        {
            authorization = Some(value.trim().to_owned());
        }
        line.clear();
    }
    let valid = format!(
        "Basic {}",
        data_encoding::BASE64.encode(format!("{PROXY_USERNAME}:{TOKEN}").as_bytes())
    );
    let tunnels = match behaviour {
        Behaviour::Permissive => true,
        Behaviour::RefuseAll => false,
        Behaviour::Strict => match authorization {
            Some(given) if given == valid => true,
            Some(_) => {
                audit.send(Reason::TokenRefused).unwrap();
                false
            }
            None => false,
        },
    };
    let answer: &[u8] = if tunnels {
        b"HTTP/1.1 200 Connection Established\r\n\r\n"
    } else {
        b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"fixture\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    };
    let _ = reader.get_mut().write_all(answer);
}
