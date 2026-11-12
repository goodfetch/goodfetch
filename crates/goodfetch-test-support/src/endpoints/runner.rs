//! `EndpointRunner` (ADR 0067): applies every row of a listener's column of
//! the committed endpoint table to that listener, one attempt per row,
//! asserts each answer, and checks each refusal in the listener's Audit log
//! through the product's verifier. It first tries the valid `hello` or
//! request, the positive control, so a listener that refuses everything,
//! or a wrong address, fails instead of passing every row. No row is ever
//! skipped: a row whose attempt cannot be built fails.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;

use serde_json::{Map, Value};

use super::connection::ListenerAddress;
use super::table::{EndpointTable, Expected, ListenerKind, Row, Rule, Transport};
use super::{http, http_proxy, json_rpc, websocket};

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
    /// An HTTP proxy on loopback TCP that tunnels to `target` for the
    /// `Proxy-Authorization: Basic` credentials of `username`, whose
    /// password is the token.
    HttpProxy {
        /// Where it listens.
        address: SocketAddr,
        /// The username of its credentials.
        username: String,
        /// The `host:port` the valid `CONNECT` tunnels to.
        target: String,
    },
}

/// The listener under test, as a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    kind: ListenerKind,
    endpoint: Endpoint,
    token: String,
    /// The token each row that presents a token of its own presents.
    given: BTreeMap<Rule, String>,
}

impl Listener {
    /// A listener of `kind` at `endpoint` that accepts `token`.
    pub fn new(kind: ListenerKind, endpoint: Endpoint, token: impl Into<String>) -> Self {
        Self {
            kind,
            endpoint,
            token: token.into(),
            given: BTreeMap::new(),
        }
    }

    /// The valid token of another listener, which the
    /// `token_from_another_listener` row presents.
    #[must_use]
    pub fn with_other_listener_token(self, token: impl Into<String>) -> Self {
        self.giving(Rule::TokenFromAnotherListener, token)
    }

    /// The token of the listener's earlier launch, which the
    /// `token_expired` row presents.
    #[must_use]
    pub fn with_expired_token(self, token: impl Into<String>) -> Self {
        self.giving(Rule::TokenExpired, token)
    }

    /// A token the listener took until it was replaced, which the
    /// `token_revoked` row presents.
    #[must_use]
    pub fn with_revoked_token(self, token: impl Into<String>) -> Self {
        self.giving(Rule::TokenRevoked, token)
    }

    /// A token of the same Daemon scoped to another listener or use, which
    /// the `token_scoped_elsewhere` row presents.
    #[must_use]
    pub fn with_scoped_elsewhere_token(self, token: impl Into<String>) -> Self {
        self.giving(Rule::TokenScopedElsewhere, token)
    }

    fn giving(mut self, rule: Rule, token: impl Into<String>) -> Self {
        self.given.insert(rule, token.into());
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
            (Expected::Accepted, observed) => observed.accepts(),
            (Expected::Challenge, Self::Answered { status, .. }) => status == 407,
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
    /// The row presents a token of its own, such as another listener's or
    /// an expired one, which the listener's description lacks.
    TokenNotGiven,
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
    /// No row of the listener's column cites the advisory.
    #[error("no row of {listener:?}'s column cites {advisory}")]
    NoRowCites {
        /// The listener's kind.
        listener: ListenerKind,
        /// The advisory.
        advisory: String,
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
        self.run_rows(listener, |_| true)
    }

    /// Applies the rows of `listener`'s column that cite `advisory`, such
    /// as `CVE-2025-49596`: its regression test. A column with no such row
    /// fails rather than passing nothing.
    pub fn run_advisory(
        &mut self,
        listener: &Listener,
        advisory: &str,
    ) -> Result<(), EndpointRunError> {
        let cited = |row: &Row| row.advisories.iter().any(|cited| cited == advisory);
        if !self
            .table
            .rules
            .values()
            .any(|row| cited(row) && row.listeners.contains_key(&listener.kind))
        {
            return Err(EndpointRunError::NoRowCites {
                listener: listener.kind,
                advisory: advisory.to_owned(),
            });
        }
        self.run_rows(listener, cited)
    }

    fn run_rows(
        &mut self,
        listener: &Listener,
        chosen: impl Fn(&Row) -> bool,
    ) -> Result<(), EndpointRunError> {
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
                Endpoint::HttpProxy {
                    address,
                    username,
                    target,
                },
                Transport::HttpProxy,
            ) => Reach::HttpProxy(ProxyReach {
                address: *address,
                username,
                target,
            }),
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
            if !chosen(row) {
                continue;
            }
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
        Rule::TokenFromAnotherListener
        | Rule::TokenExpired
        | Rule::TokenRevoked
        | Rule::TokenScopedElsewhere => {
            let Some(given) = listener.given.get(&rule) else {
                return Some(Failure::TokenNotGiven);
            };
            reach.attempt(token, Attempt::Given(given))
        }
        Rule::TokenInQueryString => reach.attempt(token, Attempt::TokenInQuery),
        Rule::HostForeign => reach.attempt(token, Attempt::ForeignHost),
        Rule::OriginForeign => reach.attempt(token, Attempt::ForeignOrigin),
        Rule::OriginMissing => reach.attempt(token, Attempt::NoOrigin),
        Rule::OriginMissingWithoutHeaderToken => reach.attempt(token, Attempt::TokenInCookie),
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
    /// The valid message or request with a token the listener's
    /// description gives a row: another listener's, an expired, a revoked
    /// or an out-of-scope one.
    Given(&'a str),
    /// An HTTP request carrying the valid token in its query string only.
    TokenInQuery,
    /// An HTTP request with the valid token and a foreign `Host`.
    ForeignHost,
    /// An HTTP request or WebSocket handshake with the valid token and a
    /// foreign `Origin`.
    ForeignOrigin,
    /// A WebSocket handshake or a request without an `Origin`, then the
    /// valid `hello`, or with the valid header token.
    NoOrigin,
    /// An HTTP request without an `Origin` whose valid token is only a
    /// cookie.
    TokenInCookie,
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
    HttpProxy(ProxyReach<'a>),
}

/// An HTTP proxy, and the `CONNECT` target and username of its valid
/// attempt.
struct ProxyReach<'a> {
    address: SocketAddr,
    username: &'a str,
    target: &'a str,
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
                    Attempt::Given(token) => hello.message("hello", Some(token)),
                    Attempt::NotHello => hello.message(NOT_HELLO, Some(token)),
                    Attempt::Valid
                    | Attempt::TokenInQuery
                    | Attempt::ForeignHost
                    | Attempt::ForeignOrigin
                    | Attempt::NoOrigin
                    | Attempt::Probe(_)
                    | Attempt::TokenInCookie => hello.message("hello", Some(token)),
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
                };
                let mut message = hello.message("hello", Some(token));
                match attempt {
                    Attempt::Valid | Attempt::Probe(_) | Attempt::TokenInCookie => {}
                    Attempt::Token(token) => message = hello.message("hello", token),
                    Attempt::Given(token) => message = hello.message("hello", Some(token)),
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
            Self::HttpProxy(reach) => reach.attempt(token, attempt),
            Self::Http {
                address,
                protected_path,
            } => {
                let authority = address.to_string();
                let cookie = format!("token={token}");
                let mut request = http::Request {
                    target: (*protected_path).to_owned(),
                    host: &authority,
                    bearer: Some(token),
                    origin: None,
                    cookie: None,
                };
                match attempt {
                    Attempt::Valid | Attempt::NotHello | Attempt::NoOrigin => {}
                    Attempt::TokenInCookie => {
                        request.bearer = None;
                        request.cookie = Some(&cookie);
                    }
                    Attempt::Token(token) => request.bearer = token,
                    Attempt::Given(token) => request.bearer = Some(token),
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

impl ProxyReach<'_> {
    /// Tries `attempt` with the valid password `token`.
    fn attempt(&self, token: &str, attempt: Attempt<'_>) -> Observed {
        let password = match attempt {
            Attempt::Token(token) => token,
            Attempt::Given(token) => Some(token),
            Attempt::Valid
            | Attempt::TokenInQuery
            | Attempt::ForeignHost
            | Attempt::ForeignOrigin
            | Attempt::NoOrigin
            | Attempt::TokenInCookie
            | Attempt::NotHello
            | Attempt::Probe(_) => Some(token),
        };
        http_proxy::attempt(
            self.address,
            self.target,
            password.map(|password| (self.username, password)),
        )
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
