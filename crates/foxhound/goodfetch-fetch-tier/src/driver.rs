//! The fetch driver (ADR 0129): what every tier that fetches a document
//! shares, above the one primitive each fills, [`Transport::send`], which
//! sends one hop and answers its head, its body still arriving. The driver
//! bounds the hop by the engine's timeouts (`timeout.first_byte` for its
//! head, `timeout.navigation` for all of it), reads its body, decodes its
//! content codings as Chrome does (ADR 0119) and owns the transport-failure
//! enum, [`TransportFailure`], with the Error code each variant ends an
//! attempt with (ADR 0118). Cancelling a fetch is dropping it. Redirects,
//! resends, retries and cookies join it with their tickets (#762, #763).

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::egress::Refused;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::page::{NAVIGATION_FAILED, NavigationFailedDetails};
use goodfetch_protocol::settings::{TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use url::Url;

use crate::decoding;

/// One hop of a fetch: one request as it leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hop {
    /// Its URL.
    pub url: Url,
    /// Its method.
    pub method: http::Method,
    /// Its headers, in order.
    pub headers: Vec<(String, String)>,
    /// Its body, if any.
    pub body: Option<Bytes>,
}

impl Hop {
    /// A `GET` of `url` with no header of its own.
    pub fn get(url: Url) -> Self {
        Self {
            url,
            method: http::Method::GET,
            headers: Vec::new(),
            body: None,
        }
    }
}

/// A boxed future the primitives answer, so a transport stays object-safe.
pub type Sending<'a, T> = Pin<Box<dyn Future<Output = Result<T, TransportFailure>> + Send + 'a>>;

/// A body still arriving, as a transport reads it.
pub trait Body: Send {
    /// Its next bytes; `None` once it ended whole.
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>>;
}

/// A hop's answer, its head read and its body still arriving.
pub struct Sent {
    /// Its status.
    pub status: u16,
    /// Its headers, their names lower-cased, in order.
    pub headers: Vec<(String, String)>,
    /// Its body, exactly as it arrives: no content coding decoded.
    pub body: Box<dyn Body>,
}

impl std::fmt::Debug for Sent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Sent")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

/// The primitive a tier that sends HTTP itself fills (ADR 0129): it sends
/// `hop` once and answers its head. It follows no redirect, decodes no
/// content coding and sets no timeout of its own: the driver does each.
pub trait Transport: Send + Sync {
    /// Sends `hop` and answers its head.
    fn send<'a>(&'a self, hop: &'a Hop) -> Sending<'a, Sent>;
}

/// Why a hop got no whole answer: each transport's library, TLS and HTTP
/// failures, translated where the transport meets its library (ADR 0127),
/// each named after the network error Chrome gives the same failure, so the
/// browser tier and the others report one failure alike.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportFailure {
    /// The egress policy refused the request or an address its host
    /// resolved to (ADRs 0014, 0017): `egress_refused`.
    Egress(Refused),
    /// The host's name resolved to no address (`net::ERR_NAME_NOT_RESOLVED`).
    NameNotResolved,
    /// The host refused the connection (`net::ERR_CONNECTION_REFUSED`).
    ConnectionRefused,
    /// The connection timed out (`net::ERR_CONNECTION_TIMED_OUT`).
    ConnectionTimedOut,
    /// The connection was reset (`net::ERR_CONNECTION_RESET`).
    ConnectionReset,
    /// The connection closed before the answer was whole
    /// (`net::ERR_CONNECTION_CLOSED`).
    ConnectionClosed,
    /// The host's certificate failed verification
    /// (`net::ERR_CERT_AUTHORITY_INVALID`).
    CertificateInvalid,
    /// The TLS handshake failed otherwise (`net::ERR_SSL_PROTOCOL_ERROR`).
    Tls,
    /// The answer's head is no valid HTTP: a line without a colon, a bare
    /// CR or a NUL in a value, which Goodfetch refuses on every tier it
    /// reads heads itself (`net::ERR_INVALID_HTTP_RESPONSE`).
    InvalidResponse,
    /// The proxy refused the tunnel `CONNECT` asked for (ADR 0053), a
    /// refusal of the proxy's own, never the site's
    /// (`net::ERR_TUNNEL_CONNECTION_FAILED`).
    TunnelRefused,
    /// The proxy asked for credentials before it opens the tunnel (ADR
    /// 0053; `net::ERR_PROXY_AUTH_REQUESTED`).
    ProxyAuthenticationRequired,
    /// The proxy itself could not be reached
    /// (`net::ERR_PROXY_CONNECTION_FAILED`).
    ProxyUnreachable,
    /// A content coding the body is labelled with did not decode
    /// (`net::ERR_CONTENT_DECODING_FAILED`, ADR 0119).
    ContentDecodingFailed,
    /// Any other failure: the generic case (ADR 0118; `net::ERR_FAILED`).
    Other,
}

impl TransportFailure {
    /// The network error Chrome names the same failure with.
    pub fn network_error(&self) -> &'static str {
        match self {
            Self::Egress(_) => "net::ERR_BLOCKED_BY_CLIENT",
            Self::NameNotResolved => "net::ERR_NAME_NOT_RESOLVED",
            Self::ConnectionRefused => "net::ERR_CONNECTION_REFUSED",
            Self::ConnectionTimedOut => "net::ERR_CONNECTION_TIMED_OUT",
            Self::ConnectionReset => "net::ERR_CONNECTION_RESET",
            Self::ConnectionClosed => "net::ERR_CONNECTION_CLOSED",
            Self::CertificateInvalid => "net::ERR_CERT_AUTHORITY_INVALID",
            Self::Tls => "net::ERR_SSL_PROTOCOL_ERROR",
            Self::InvalidResponse => "net::ERR_INVALID_HTTP_RESPONSE",
            Self::TunnelRefused => "net::ERR_TUNNEL_CONNECTION_FAILED",
            Self::ProxyAuthenticationRequired => "net::ERR_PROXY_AUTH_REQUESTED",
            Self::ProxyUnreachable => "net::ERR_PROXY_CONNECTION_FAILED",
            Self::ContentDecodingFailed => "net::ERR_CONTENT_DECODING_FAILED",
            Self::Other => "net::ERR_FAILED",
        }
    }

    /// What it ends the attempt fetching `url` with: `egress_refused` for
    /// the egress policy's refusal, and `navigation_failed` with its network
    /// error for every other failure, retryable (ADR 0118).
    pub fn error(&self, url: &str) -> CallError {
        if let Self::Egress(refused) = self {
            return refused.error();
        }
        let error = self.network_error();
        CallError::new(
            &NAVIGATION_FAILED,
            format!("The fetch of {url} failed: {error}."),
            &NavigationFailedDetails {
                url: url.to_owned(),
                error: Some(error.to_owned()),
            },
        )
    }
}

/// A fetched document: where it ended, its status and headers, and its body
/// with its content codings decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    /// The URL that answered.
    pub url: Url,
    /// Its status.
    pub status: u16,
    /// Its headers, their names lower-cased, in order.
    pub headers: Vec<(String, String)>,
    /// Its body, decoded as its `Content-Encoding` lists.
    pub body: Vec<u8>,
}

/// The engine's timeouts a fetch waits under.
#[derive(Clone, Debug, PartialEq)]
pub struct Deadlines {
    /// `timeout.first_byte`: until the answer's head.
    pub first_byte: Deadline,
    /// `timeout.navigation`: until the whole document.
    pub navigation: Deadline,
}

impl Deadlines {
    /// The timeouts a Run's resolved settings give, whose keys always hold
    /// a duration.
    pub fn of(settings: &Resolved) -> Self {
        Self {
            first_byte: Deadline::of(settings, &TIMEOUT_FIRST_BYTE)
                .expect("invariant: `timeout.first_byte` resolves to a duration"),
            navigation: Deadline::of(settings, &TIMEOUT_NAVIGATION)
                .expect("invariant: `timeout.navigation` resolves to a duration"),
        }
    }
}

/// Fetches `hop` through `transport` under `deadlines` on `clock`: its head
/// within `timeout.first_byte`, its whole body within `timeout.navigation`,
/// then its content codings decoded.
///
/// # Errors
///
/// `deadline_exceeded` naming the timeout that passed, or the transport
/// failure's own error ([`TransportFailure::error`]).
///
/// # Cancel safety
///
/// Dropping the future closes the hop's connection.
pub async fn fetch(
    transport: &dyn Transport,
    hop: &Hop,
    deadlines: &Deadlines,
    clock: &Clock,
) -> Result<Fetched, CallError> {
    let url = hop.url.as_str();
    let whole = async {
        let mut sent = deadlines
            .first_byte
            .within(clock, "first_byte", transport.send(hop))
            .await?
            .map_err(|failure| failure.error(url))?;
        let mut body = Vec::new();
        while let Some(chunk) = sent
            .body
            .chunk()
            .await
            .map_err(|failure| failure.error(url))?
        {
            body.extend_from_slice(&chunk);
        }
        let codings = sent
            .headers
            .iter()
            .filter(|(name, _)| name == "content-encoding")
            .map(|(_, value)| value.as_str());
        let body = decoding::decode(codings, body)
            .map_err(|_| TransportFailure::ContentDecodingFailed.error(url))?;
        Ok::<Fetched, CallError>(Fetched {
            url: hop.url.clone(),
            status: sent.status,
            headers: sent.headers,
            body,
        })
    };
    deadlines
        .navigation
        .within(clock, "navigation", whole)
        .await?
}
