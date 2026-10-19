//! Fixture listeners for the endpoint runner's tests: a JSON-RPC stream
//! listener and an HTTP ops listener, each strict (it refuses every row as
//! the table says and records each refusal in its own Audit log, a channel),
//! permissive (it accepts everything) or refusing everything.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use goodfetch_test_support::Rule;

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
}

/// A fixture listener's Audit log, read as a product's verifier reads its
/// own: each refusal's entry is the next one.
pub(super) struct AuditLog(Receiver<Reason>);

impl AuditLog {
    /// Whether the next entry records `rule`'s refusal.
    pub(super) fn records(&mut self, rule: Rule) -> bool {
        let expected = match rule {
            Rule::TokenMissing => Reason::TokenMissing,
            Rule::TokenWrong | Rule::TokenFromAnotherListener => Reason::TokenRefused,
            Rule::TokenInQueryString => Reason::TokenInQuery,
            Rule::HostForeign => Reason::ForeignHost,
            Rule::OriginForeign => Reason::ForeignOrigin,
            Rule::HelloNotFirst => Reason::NotHello,
            Rule::ProbesAnswerStatusOnly => return false,
        };
        self.0.recv_timeout(Duration::from_secs(5)) == Ok(expected)
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

fn serve_json_rpc<S>(stream: S, behaviour: Behaviour, audit: &Sender<Reason>)
where
    S: std::io::Read + std::io::Write,
{
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let refusal = match behaviour {
        Behaviour::Permissive => None,
        Behaviour::RefuseAll => return,
        Behaviour::Strict => {
            let message: serde_json::Value = serde_json::from_str(&line).unwrap();
            if message["method"] == "hello" {
                match message["params"]["token"].as_str() {
                    None => Some(Reason::TokenMissing),
                    Some(TOKEN) => None,
                    Some(_) => Some(Reason::TokenRefused),
                }
            } else {
                Some(Reason::NotHello)
            }
        }
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
