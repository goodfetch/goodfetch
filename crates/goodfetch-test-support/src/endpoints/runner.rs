//! `EndpointRunner` (ADR 0067): applies every row of a listener's column of
//! the committed endpoint table to that listener, one attempt per row,
//! asserts each answer, and checks each refusal in the listener's Audit log
//! through the product's verifier. It first tries the valid `hello` or
//! request, the positive control, so a listener that refuses everything,
//! or a wrong address, fails instead of passing every row. No row is ever
//! skipped: a row whose attempt cannot be built fails.

use std::fmt;
use std::net::SocketAddr;

use serde_json::{Map, Value};

use super::connection::ListenerAddress;
use super::table::{EndpointTable, Expected, ListenerKind, Rule, Transport};
use super::{http, json_rpc, websocket};

/// The method of the first message that is not a `hello`.
const NOT_HELLO: &str = "ping";
/// The foreign host a DNS-rebinding page's request names (RFC 2606).
const FOREIGN_HOST: &str = "foreign.invalid";
/// A foreign page's origin.
const FOREIGN_ORIGIN: &str = "https://foreign.invalid";
/// The ops listener's probes (ADR 0050).
const PROBES: [&str; 2] = ["/livez", "/readyz"];

/// The `hello` a JSON-RPC stream listener accepts, without its token: its
/// params, and the field of the params that carries the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    params: Map<String, Value>,
    token_field: String,
}

impl Hello {
    /// A `hello` with these params, whose `token_field` the runner sets to
    /// each attempt's token.
    pub fn new(params: Map<String, Value>, token_field: impl Into<String>) -> Self {
        Self {
            params,
            token_field: token_field.into(),
        }
    }

    /// The first message: a call of `method` with the params and `token`.
    fn message(&self, method: &str, token: Option<&str>) -> Value {
        let mut params = self.params.clone();
        params.remove(&self.token_field);
        if let Some(token) = token {
            params.insert(self.token_field.clone(), Value::String(token.to_owned()));
        }
        serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }
}

/// How the runner reaches a listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A newline-delimited JSON-RPC stream that opens with `hello`.
    JsonRpc {
        /// Where it listens.
        address: ListenerAddress,
        /// The `hello` it accepts.
        hello: Hello,
    },
    /// An HTTP/1.1 listener on loopback TCP.
    Http {
        /// Where it listens.
        address: SocketAddr,
    },
    /// A WebSocket listener on loopback TCP, whose valid handshake names
    /// `address` as its `Host` and `http://<address>` as its `Origin`, and
    /// whose first message is a `hello`.
    WebSocket {
        /// Where it listens.
        address: SocketAddr,
        /// The `hello` it accepts.
        hello: Hello,
    },
    /// A WebSocket listener on loopback TCP whose valid handshake names
    /// `address` as its `Host` and `http://<address>` as its `Origin` and
    /// carries the token as the cookie `cookie`, and whose first message
    /// is a `hello` with these params, which carry no token.
    SignedInWebSocket {
        /// Where it listens.
        address: SocketAddr,
        /// The sign-in cookie's name.
        cookie: String,
        /// The `hello`'s params.
        hello: Map<String, Value>,
    },
}

/// The listener under test, as a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    kind: ListenerKind,
    endpoint: Endpoint,
    token: String,
    other_listener_token: Option<String>,
}

impl Listener {
    /// A listener of `kind` at `endpoint` that accepts `token`.
    pub fn new(kind: ListenerKind, endpoint: Endpoint, token: impl Into<String>) -> Self {
        Self {
            kind,
            endpoint,
            token: token.into(),
            other_listener_token: None,
        }
    }

    /// The valid token of another listener, which the
    /// `token_from_another_listener` row presents.
    #[must_use]
    pub fn with_other_listener_token(mut self, token: impl Into<String>) -> Self {
        self.other_listener_token = Some(token.into());
        self
    }
}

/// The product's reader of the listener's Audit log.
pub trait AuditVerifier {
    /// Whether the Audit log records the listener's refusal of `rule`'s
    /// attempt, asked once the listener has answered it.
    fn records(&mut self, listener: ListenerKind, rule: Rule) -> Result<bool, AuditUnreadable>;
}

impl<F> AuditVerifier for F
where
    F: FnMut(ListenerKind, Rule) -> Result<bool, AuditUnreadable>,
{
    fn records(&mut self, listener: ListenerKind, rule: Rule) -> Result<bool, AuditUnreadable> {
        self(listener, rule)
    }
}

/// The verifier could not read the Audit log.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the Audit log cannot be read: {message}")]
pub struct AuditUnreadable {
    message: String,
}

impl AuditUnreadable {
    /// Why the Audit log cannot be read.
    pub fn new(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// What the listener did with an attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Observed {
    /// A JSON-RPC stream listener replied with a `result`.
    Replied,
    /// The listener closed the connection without answering.
    Closed,
    /// The listener neither answered nor closed before the deadline.
    StayedOpen,
    /// An HTTP listener answered with this status and body length.
    Answered {
        /// The status.
        status: u16,
        /// The body's length in bytes.
        body_length: usize,
    },
    /// The answer was no JSON-RPC line or HTTP response.
    Malformed,
    /// The runner could not reach the listener.
    Unreachable {
        /// The connection's error.
        error: std::io::ErrorKind,
    },
}

impl Observed {
    /// Whether this answer is the cell's.
    fn meets(self, expected: Expected) -> bool {
        match (expected, self) {
            (Expected::Closed, Self::Closed) => true,
            (
                Expected::Status { status },
                Self::Answered {
                    status: observed, ..
                },
            ) => observed == status,
            (Expected::StatusOnly, Self::Answered { body_length, .. }) => body_length == 0,
            _ => false,
        }
    }

    /// Whether this answer accepts the valid `hello` or request.
    fn accepts(self) -> bool {
        match self {
            Self::Replied => true,
            Self::Answered { status, .. } => (200..300).contains(&status),
            _ => false,
        }
    }
}

/// Why a row failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Failure {
    /// The listener's answer is not the cell's.
    Unexpected {
        /// The cell.
        expected: Expected,
        /// What the listener did.
        observed: Observed,
    },
    /// The listener refused, but its Audit log does not record it.
    NotAudited,
    /// The verifier could not read the Audit log.
    AuditUnreadable {
        /// Why.
        message: String,
    },
    /// The row presents another listener's token, which the listener's
    /// description lacks.
    NoOtherListenerToken,
}

/// A row the listener failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowFailure {
    /// The row's rule.
    pub rule: Rule,
    /// Why it failed.
    pub failure: Failure,
}

/// Why a listener fails the endpoint table.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EndpointRunError {
    /// The listener's endpoint is not its kind's transport.
    #[error("{listener:?} is described with another kind's transport")]
    TransportMismatch {
        /// The listener's kind.
        listener: ListenerKind,
    },
    /// The listener did not accept the valid `hello` or request.
    #[error("the listener did not accept the valid hello or request: {observed:?}")]
    ControlRefused {
        /// What it did.
        observed: Observed,
    },
    /// Rows the listener failed.
    #[error("the listener failed {} rows of the endpoint table: {failures:?}", failures.len())]
    Rows {
        /// Each failed row, in the table's order.
        failures: Vec<RowFailure>,
    },
}

/// Applies the committed endpoint table to listeners, checking refusals
/// through `A`, the product's Audit log verifier.
pub struct EndpointRunner<A> {
    table: EndpointTable,
    audit: A,
}

impl<A: AuditVerifier> EndpointRunner<A> {
    /// A runner over the committed table.
    pub fn new(audit: A) -> Self {
        Self {
            table: EndpointTable::committed(),
            audit,
        }
    }

    /// Applies every row of `listener`'s column to it.
    pub fn run(&mut self, listener: &Listener) -> Result<(), EndpointRunError> {
        let reach = match (&listener.endpoint, listener.kind.transport()) {
            (Endpoint::JsonRpc { address, hello }, Transport::JsonRpcStream) => {
                Reach::JsonRpc { address, hello }
            }
            (Endpoint::Http { address }, Transport::Http { protected_path }) => Reach::Http {
                address: *address,
                protected_path,
            },
            (Endpoint::WebSocket { address, hello }, Transport::WebSocket) => Reach::WebSocket {
                address: *address,
                hello,
            },
            (
                Endpoint::SignedInWebSocket {
                    address,
                    cookie,
                    hello,
                },
                Transport::SignedInWebSocket { path },
            ) => Reach::SignedInWebSocket {
                address: *address,
                path,
                cookie,
                hello,
            },
            _ => {
                return Err(EndpointRunError::TransportMismatch {
                    listener: listener.kind,
                });
            }
        };
        let control = reach.attempt(&listener.token, Attempt::Valid);
        if !control.accepts() {
            return Err(EndpointRunError::ControlRefused { observed: control });
        }
        let mut failures = Vec::new();
        for (&rule, row) in &self.table.rules {
            let Some(&expected) = row.listeners.get(&listener.kind) else {
                continue;
            };
            if let Some(failure) = apply(&mut self.audit, listener, &reach, rule, expected) {
                failures.push(RowFailure { rule, failure });
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(EndpointRunError::Rows { failures })
        }
    }
}

/// Tries `rule`'s attempt on `listener`, and checks a refusal through
/// `audit`.
fn apply(
    audit: &mut impl AuditVerifier,
    listener: &Listener,
    reach: &Reach<'_>,
    rule: Rule,
    expected: Expected,
) -> Option<Failure> {
    let token = listener.token.as_str();
    let observed = match rule {
        Rule::TokenMissing => reach.attempt(token, Attempt::Token(None)),
        Rule::TokenWrong => reach.attempt(token, Attempt::Token(Some(&wrong_token(token)))),
        Rule::TokenFromAnotherListener => {
            let Some(other) = &listener.other_listener_token else {
                return Some(Failure::NoOtherListenerToken);
            };
            reach.attempt(token, Attempt::Token(Some(other)))
        }
        Rule::TokenInQueryString => reach.attempt(token, Attempt::TokenInQuery),
        Rule::HostForeign => reach.attempt(token, Attempt::ForeignHost),
        Rule::OriginForeign => reach.attempt(token, Attempt::ForeignOrigin),
        Rule::OriginMissing => reach.attempt(token, Attempt::NoOrigin),
        Rule::HelloNotFirst => reach.attempt(token, Attempt::NotHello),
        Rule::ProbesAnswerStatusOnly => {
            return PROBES
                .iter()
                .map(|path| reach.attempt(token, Attempt::Probe(path)))
                .find(|observed| !observed.meets(expected))
                .map(|observed| Failure::Unexpected { expected, observed });
        }
    };
    if !observed.meets(expected) {
        return Some(Failure::Unexpected { expected, observed });
    }
    if !expected.is_refusal() {
        return None;
    }
    match audit.records(listener.kind, rule) {
        Ok(true) => None,
        Ok(false) => Some(Failure::NotAudited),
        Err(error) => Some(Failure::AuditUnreadable {
            message: error.message,
        }),
    }
}

/// One attempt on a listener.
#[derive(Clone, Copy)]
enum Attempt<'a> {
    /// The valid `hello` or request.
    Valid,
    /// The valid message or request with this token, or none.
    Token(Option<&'a str>),
    /// An HTTP request carrying the valid token in its query string only.
    TokenInQuery,
    /// An HTTP request with the valid token and a foreign `Host`.
    ForeignHost,
    /// An HTTP request or WebSocket handshake with the valid token and a
    /// foreign `Origin`.
    ForeignOrigin,
    /// A WebSocket handshake without an `Origin`, then the valid `hello`.
    NoOrigin,
    /// A first message with the valid token that is no `hello`.
    NotHello,
    /// A request without a token for one of the ops listener's probes.
    Probe(&'a str),
}

/// A listener's endpoint, with what its kind's transport adds.
enum Reach<'a> {
    JsonRpc {
        address: &'a ListenerAddress,
        hello: &'a Hello,
    },
    Http {
        address: SocketAddr,
        protected_path: &'static str,
    },
    WebSocket {
        address: SocketAddr,
        hello: &'a Hello,
    },
    SignedInWebSocket {
        address: SocketAddr,
        path: &'static str,
        cookie: &'a str,
        hello: &'a Map<String, Value>,
    },
}

impl Reach<'_> {
    /// Tries `attempt` on a listener whose valid token is `token`. The
    /// table's check keeps each rule to the transports that can carry its
    /// attempt, so an attempt a transport cannot carry is never asked for.
    fn attempt(&self, token: &str, attempt: Attempt<'_>) -> Observed {
        match self {
            Self::JsonRpc { address, hello } => {
                let message = match attempt {
                    Attempt::Token(token) => hello.message("hello", token),
                    Attempt::NotHello => hello.message(NOT_HELLO, Some(token)),
                    Attempt::Valid
                    | Attempt::TokenInQuery
                    | Attempt::ForeignHost
                    | Attempt::ForeignOrigin
                    | Attempt::NoOrigin
                    | Attempt::Probe(_) => hello.message("hello", Some(token)),
                };
                json_rpc::attempt(address, &message)
            }
            Self::WebSocket { address, hello } => {
                let authority = address.to_string();
                let origin = format!("http://{authority}");
                let mut handshake = websocket::Handshake {
                    target: "/".to_owned(),
                    host: &authority,
                    origin: Some(&origin),
                    cookie: None,
                };
                let mut message = hello.message("hello", Some(token));
                match attempt {
                    Attempt::Valid | Attempt::Probe(_) => {}
                    Attempt::Token(token) => message = hello.message("hello", token),
                    Attempt::TokenInQuery => {
                        handshake.target = format!("/?access_token={}", percent_encoded(token));
                        message = hello.message("hello", None);
                    }
                    Attempt::ForeignHost => handshake.host = FOREIGN_HOST,
                    Attempt::ForeignOrigin => handshake.origin = Some(FOREIGN_ORIGIN),
                    Attempt::NoOrigin => handshake.origin = None,
                    Attempt::NotHello => message = hello.message(NOT_HELLO, Some(token)),
                }
                websocket::attempt(*address, &handshake, &message)
            }
            Self::SignedInWebSocket {
                address,
                path,
                cookie,
                hello,
            } => {
                let authority = address.to_string();
                let origin = format!("http://{authority}");
                let signed_in = |token: &str| format!("{cookie}={token}");
                let mut handshake = websocket::Handshake {
                    target: (*path).to_owned(),
                    host: &authority,
                    origin: Some(&origin),
                    cookie: Some(signed_in(token)),
                };
                let first = |method: &str| serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": hello });
                let mut message = first("hello");
                match attempt {
                    Attempt::Valid | Attempt::Probe(_) => {}
                    Attempt::Token(token) => handshake.cookie = token.map(signed_in),
                    Attempt::TokenInQuery => {
                        handshake.target =
                            format!("{path}?access_token={}", percent_encoded(token));
                        handshake.cookie = None;
                    }
                    Attempt::ForeignHost => handshake.host = FOREIGN_HOST,
                    Attempt::ForeignOrigin => handshake.origin = Some(FOREIGN_ORIGIN),
                    Attempt::NoOrigin => handshake.origin = None,
                    Attempt::NotHello => message = first(NOT_HELLO),
                }
                websocket::attempt(*address, &handshake, &message)
            }
            Self::Http {
                address,
                protected_path,
            } => {
                let authority = address.to_string();
                let mut request = http::Request {
                    target: (*protected_path).to_owned(),
                    host: &authority,
                    bearer: Some(token),
                    origin: None,
                };
                match attempt {
                    Attempt::Valid | Attempt::NotHello | Attempt::NoOrigin => {}
                    Attempt::Token(token) => request.bearer = token,
                    Attempt::TokenInQuery => {
                        request.target =
                            format!("{protected_path}?access_token={}", percent_encoded(token));
                        request.bearer = None;
                    }
                    Attempt::ForeignHost => request.host = FOREIGN_HOST,
                    Attempt::ForeignOrigin => request.origin = Some(FOREIGN_ORIGIN),
                    Attempt::Probe(path) => {
                        path.clone_into(&mut request.target);
                        request.bearer = None;
                    }
                }
                http::attempt(*address, &request)
            }
        }
    }
}

/// A token of the same length and alphabet as `token` that differs from it
/// in every letter and digit.
fn wrong_token(token: &str) -> String {
    let wrong: String = token
        .chars()
        .map(|character| match character {
            '9' => '0',
            'z' => 'a',
            'Z' => 'A',
            '0'..='8' | 'a'..='y' | 'A'..='Y' => {
                char::from_u32(u32::from(character) + 1).unwrap_or(character)
            }
            other => other,
        })
        .collect();
    if wrong == token {
        format!("{token}x")
    } else {
        wrong
    }
}

/// `value` with every byte outside RFC 3986's unreserved set
/// percent-encoded.
fn percent_encoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
