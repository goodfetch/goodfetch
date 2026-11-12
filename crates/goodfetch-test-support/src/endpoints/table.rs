//! The endpoint table's format (ADR 0067): one row per refusal rule, one
//! cell per listener the rule applies to, saying what that listener must do
//! with the attempt. The committed table lives among test support's shared
//! fixtures, `test-support/fixtures/endpoint-table.toml`; its JSON Schema is
//! generated from these types. Reading a table also checks it: every row
//! applies to a listener, every listener has a row, and every cell is one
//! its listener's transport can try.

use std::collections::BTreeMap;

/// The committed table, which `EndpointRunner` applies.
pub(super) const COMMITTED: &str =
    include_str!("../../../../test-support/fixtures/endpoint-table.toml");

/// A listener with a column in the endpoint table. Listeners join as their
/// tickets land.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ListenerKind {
    /// The Daemon's protocol socket: a `0600` Unix socket, or loopback TCP
    /// on Windows (ADR 0024).
    DaemonSocket,
    /// An Attached engine's private endpoint, for its workers and the
    /// Daemon's forwarded live actions (ADR 0031).
    PrivateEndpoint,
    /// The Daemon's ops listener: its probes and `/metrics` (ADR 0050).
    OpsListener,
    /// The Daemon's WebSocket: the protocol over WebSocket, its handshake
    /// held to the Daemon's `Host` and `Origin` (ADRs 0049 and 0067).
    DaemonWebsocket,
    /// An Exposed Daemon's address, reached with newline-delimited
    /// JSON-RPC over TCP (ADR 0049).
    ExposedDaemon,
    /// A Session's proxy relay (`goodfetch-page`'s `proxy::relay`): a
    /// loopback HTTP proxy of the Session's own, which tunnels only for the
    /// Session's per-Session credentials (ADR 0037 / SA-22; spec #577).
    ProxyRelay,
}

impl ListenerKind {
    /// Every listener, each of which must have a row.
    pub(super) const ALL: [Self; 6] = [
        Self::DaemonSocket,
        Self::PrivateEndpoint,
        Self::OpsListener,
        Self::DaemonWebsocket,
        Self::ExposedDaemon,
        Self::ProxyRelay,
    ];

    /// How the runner reaches this listener.
    pub(super) fn transport(self) -> Transport {
        match self {
            Self::DaemonSocket | Self::PrivateEndpoint | Self::ExposedDaemon => {
                Transport::JsonRpcStream
            }
            Self::OpsListener => Transport::Http {
                protected_path: "/metrics",
            },
            Self::DaemonWebsocket => Transport::WebSocket,
            Self::ProxyRelay => Transport::HttpProxy,
        }
    }
}

/// How a listener is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Transport {
    /// Newline-delimited JSON-RPC that opens with a `hello` carrying the
    /// token (ADR 0024).
    JsonRpcStream,
    /// HTTP/1.1, with the token as a bearer token on `protected_path`.
    Http { protected_path: &'static str },
    /// JSON-RPC over WebSocket, one message per WebSocket message, that
    /// opens with a `hello` carrying the token once its handshake is
    /// accepted (ADR 0049).
    WebSocket,
    /// An HTTP proxy's `CONNECT`, with the token as the password of its
    /// `Proxy-Authorization: Basic` credentials (RFC 9110, section 11.7.1).
    HttpProxy,
}

/// A refusal rule: a row of the endpoint table, which the runner knows how
/// to try.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The first message or the request carries no token.
    TokenMissing,
    /// The token is one the listener never issued.
    TokenWrong,
    /// The token is valid, but for another listener.
    TokenFromAnotherListener,
    /// The token's life ended with the launch that issued it: the token of
    /// the listener's earlier launch.
    TokenExpired,
    /// The token was replaced while it was in force, as `daemon restart`
    /// or an operator's new ops token replaces it.
    TokenRevoked,
    /// The token is one of the same Daemon whose scope is another, such as
    /// the ops token, read-only on `/metrics`, on a protocol listener.
    TokenScopedElsewhere,
    /// The valid token is sent in the query string.
    TokenInQueryString,
    /// The request names a foreign `Host`.
    HostForeign,
    /// The request carries a foreign `Origin`.
    OriginForeign,
    /// The request or the WebSocket handshake carries no `Origin`: an HTTP
    /// listener accepts it with a header token, which a browser page cannot
    /// send cross-site.
    OriginMissing,
    /// The request carries no `Origin` and no header token, the valid token
    /// only in a cookie, as a browser page's cross-site request could send
    /// it.
    OriginMissingWithoutHeaderToken,
    /// The connection's first message is not a `hello`.
    HelloNotFirst,
    /// The ops listener's probes answer with a status and no body.
    ProbesAnswerStatusOnly,
}

impl Rule {
    /// Whether the runner can try this rule over `transport`.
    fn tried_over(self, transport: Transport) -> bool {
        let http = matches!(transport, Transport::Http { .. });
        let websocket = transport == Transport::WebSocket;
        let proxy = transport == Transport::HttpProxy;
        match self {
            Self::TokenMissing
            | Self::TokenWrong
            | Self::TokenFromAnotherListener
            | Self::TokenExpired
            | Self::TokenRevoked
            | Self::TokenScopedElsewhere => true,
            Self::TokenInQueryString
            | Self::OriginMissing
            | Self::HostForeign
            | Self::OriginForeign => http || websocket,
            Self::ProbesAnswerStatusOnly | Self::OriginMissingWithoutHeaderToken => http,
            Self::HelloNotFirst => !http && !proxy,
        }
    }

    /// Whether a WebSocket listener refuses this rule's attempt at its
    /// handshake, with a status, rather than by closing after the first
    /// message.
    fn at_the_handshake(self) -> bool {
        matches!(
            self,
            Self::HostForeign | Self::OriginForeign | Self::OriginMissing
        )
    }
}

/// What a listener must do with a rule's attempt: a table cell.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "expect", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expected {
    /// A JSON-RPC stream or WebSocket listener closes the connection
    /// without answering.
    Closed,
    /// An HTTP listener refuses the request, or a WebSocket listener its
    /// handshake, with this status.
    Status {
        /// A 4xx status.
        #[schemars(range(min = 400, max = 499))]
        status: u16,
    },
    /// An HTTP listener answers with a status and no body. No refusal, so
    /// no Audit log entry.
    StatusOnly,
    /// An HTTP listener accepts the request, as it does the valid one, with
    /// a 2xx. No refusal, so no Audit log entry.
    Accepted,
    /// An HTTP proxy asks for credentials with a `407` (RFC 9110, section
    /// 15.5.8), as it answers every client's first request, the browser's
    /// included, before the client learns them. No refusal, so no Audit log
    /// entry; no byte is relayed.
    Challenge,
}

impl Expected {
    /// Whether the listener refuses, which its Audit log records.
    pub(super) fn is_refusal(self) -> bool {
        match self {
            Self::Closed | Self::Status { .. } => true,
            Self::StatusOnly | Self::Accepted | Self::Challenge => false,
        }
    }

    /// Whether a listener reached over `transport` can give this answer to
    /// `rule`'s attempt.
    fn fits(self, rule: Rule, transport: Transport) -> bool {
        let http = matches!(transport, Transport::Http { .. } | Transport::HttpProxy);
        let websocket = transport == Transport::WebSocket;
        let at_the_handshake = rule.at_the_handshake();
        rule.tried_over(transport)
            && match self {
                Self::Closed => {
                    transport == Transport::JsonRpcStream || (websocket && !at_the_handshake)
                }
                Self::Status { status } => {
                    (http || (websocket && at_the_handshake))
                        && (400..=499).contains(&status)
                        && rule != Rule::ProbesAnswerStatusOnly
                }
                Self::StatusOnly => http && rule == Rule::ProbesAnswerStatusOnly,
                Self::Accepted => {
                    matches!(transport, Transport::Http { .. }) && rule == Rule::OriginMissing
                }
                Self::Challenge => transport == Transport::HttpProxy && rule == Rule::TokenMissing,
            }
    }
}

/// One rule's row.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    /// What the attempt does, in one sentence.
    pub(crate) summary: String,
    /// The ADRs the rule comes from.
    pub(crate) adrs: Vec<u16>,
    /// The advisories whose bypass the row's attempt repeats, such as
    /// `CVE-2025-49596`, whose regression tests run the rows citing them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) advisories: Vec<String>,
    /// The cell of each listener the rule applies to.
    #[serde(default)]
    #[schemars(schema_with = "map_keyed_by::<ListenerKind, Expected>")]
    pub(crate) listeners: BTreeMap<ListenerKind, Expected>,
}

/// The schema of a map whose keys are `K`'s names, which JSON Schema's
/// `propertyNames` holds to them.
fn map_keyed_by<K: schemars::JsonSchema, V: schemars::JsonSchema>(
    generator: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    let mut schema = generator.subschema_for::<BTreeMap<String, V>>();
    schema.insert(
        "propertyNames".to_owned(),
        generator.subschema_for::<K>().to_value(),
    );
    schema
}

/// The endpoint table: a row per rule, a column per listener.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointTable {
    /// Each rule's row, by the rule's name.
    #[serde(rename = "rule")]
    #[schemars(schema_with = "map_keyed_by::<Rule, Row>")]
    pub(crate) rules: BTreeMap<Rule, Row>,
}

impl EndpointTable {
    /// Reads a table from its TOML source and checks it.
    pub(super) fn parse(source: &str) -> Result<Self, TableError> {
        let table: Self = toml::from_str(source).map_err(|error| TableError::Unreadable {
            message: error.to_string(),
        })?;
        table.check()?;
        Ok(table)
    }

    /// The committed table, which the Quick suite checks.
    pub(super) fn committed() -> Self {
        Self::parse(COMMITTED)
            .expect("invariant: the Quick suite checks the committed endpoint table")
    }

    fn check(&self) -> Result<(), TableError> {
        for (&rule, row) in &self.rules {
            if row.listeners.is_empty() {
                return Err(TableError::RowWithoutListener { rule });
            }
            for (&listener, expected) in &row.listeners {
                if !expected.fits(rule, listener.transport()) {
                    return Err(TableError::CellUnfit { rule, listener });
                }
            }
        }
        for listener in ListenerKind::ALL {
            if !self
                .rules
                .values()
                .any(|row| row.listeners.contains_key(&listener))
            {
                return Err(TableError::ListenerWithoutRow { listener });
            }
        }
        Ok(())
    }
}

/// Why an endpoint table fails its check.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub(super) enum TableError {
    /// The source is no table of this format.
    #[error("the endpoint table cannot be read: {message}")]
    Unreadable { message: String },
    /// A row applies to no listener.
    #[error("the endpoint table's row {rule:?} applies to no listener")]
    RowWithoutListener { rule: Rule },
    /// A listener's column has no row.
    #[error("the endpoint table's column {listener:?} has no row")]
    ListenerWithoutRow { listener: ListenerKind },
    /// A cell asks for an attempt or an answer its listener's transport
    /// cannot give.
    #[error("the endpoint table's cell {rule:?} for {listener:?} is one its listener cannot try")]
    CellUnfit { rule: Rule, listener: ListenerKind },
}

#[cfg(test)]
mod tests;
