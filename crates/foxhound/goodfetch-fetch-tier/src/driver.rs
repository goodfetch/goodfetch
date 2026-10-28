//! The fetch driver (ADR 0129): what every tier that fetches a document
//! shares, above the one primitive each fills, [`Transport::send`], which
//! sends one hop and answers its head, its body still arriving. The driver
//! bounds the hop by the engine's timeouts (`timeout.first_byte` for its
//! head, `timeout.navigation` for all of it), reads its body, decodes its
//! content codings as Chrome does (ADR 0119) and owns the transport-failure
//! enum, [`TransportFailure`], with the Error code each variant ends an
//! attempt with (ADR 0118). Cancelling a fetch is dropping it. A fetch is
//! one hop: the `chain` module follows a crawl request's redirects and
//! refresh stubs over it (#762); retries join with their ticket. A
//! Session that may move to the browser keeps its cookies in its browser
//! context, which browser-fetch sends from (`transports`); a Session that
//! never moves keeps none yet.
//!
//! **Cut bodies (ADR 0118; pitfall H15):** once a hop's head arrived, any
//! failure of its body, or a compressed stream that stops before its end,
//! is `response_truncated`: retryable, and the body is never handed on. A
//! body its connection's close delimits is whole at the close.
//!
//! **Resends (ADR 0118; pitfall H05):** a hop the site never processed
//! (an HTTP/2 stream refused, `REFUSED_STREAM` or a `GOAWAY` below it, an
//! HTTP/2 `HTTP_1_1_REQUIRED`, a 421, and for a read only a reused idle
//! connection that closes before answering or answers 408) is sent once
//! more on a new connection, inside the same fetch: no attempt, retry or
//! Politeness start counts it. The resend's own failure or answer is the
//! fetch's, never sent again.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;

use bytes::Bytes;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};
use goodfetch_engine::happy_eyeballs::FamilyUnavailable;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tls::{CertificateChanged, CertificateInvalid, ExceptionUsed};
use goodfetch_frontier::backoff_classes::Met;
use goodfetch_protocol::dns::IpFamily;
use goodfetch_protocol::page::{
    NAVIGATION_FAILED, NavigationFailedDetails, RESPONSE_TRUNCATED, ResponseTruncatedDetails,
};
use goodfetch_protocol::settings::{TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_protocol::tls::CertificateFailure;
use url::{Host, Url};

use crate::content_types::{Accepts, typed};
use crate::decoding::{self, DecodingFailed};

#[cfg(test)]
mod tests;

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
    /// Whether it only reads, so a site that may have seen it is sent it
    /// again after a reused connection closed (ADR 0118): a crawl
    /// request's hop whatever its method, which ADR 0106 makes a read, and
    /// any other only for a method [`Hop::idempotent`] names.
    pub read: bool,
}

impl Hop {
    /// A `GET` of `url` with no header of its own, a read.
    pub fn get(url: Url) -> Self {
        Self {
            url,
            method: http::Method::GET,
            headers: Vec::new(),
            body: None,
            read: true,
        }
    }

    /// Whether `method` is idempotent, so a request of it is a read
    /// (RFC 9110, section 9.2.2).
    pub fn idempotent(method: &http::Method) -> bool {
        [
            http::Method::GET,
            http::Method::HEAD,
            http::Method::OPTIONS,
            http::Method::TRACE,
            http::Method::PUT,
            http::Method::DELETE,
        ]
        .contains(method)
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
    /// Whether it came on an idle connection an earlier hop left, whose
    /// 408 means the site timed the connection out, not the request.
    pub reused: bool,
    /// Whether its body arrives with its content codings decoded already,
    /// as the browser hands a body over on the browser-fetch tier, which
    /// the driver then only checks (ADR 0119).
    pub decoded: bool,
    /// The invalid-certificate exception of `tls.accept_invalid_certs` its
    /// host was reached through (ADR 0042); none when its certificate
    /// verified, or it came over plain HTTP.
    pub certificate_exception: Option<ExceptionUsed>,
}

impl std::fmt::Debug for Sent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Sent")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("reused", &self.reused)
            .field("decoded", &self.decoded)
            .field("certificate_exception", &self.certificate_exception)
            .finish_non_exhaustive()
    }
}

/// Whether a hop may go out on a network connection an earlier hop left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reuse {
    /// On any of the transport's connections: an idle one an earlier hop
    /// left, or a new one.
    Allowed,
    /// On a new connection, never one an earlier hop used: a resend's (ADR
    /// 0118).
    Never,
}

/// The primitive a tier that sends HTTP itself fills (ADR 0129): it sends
/// `hop` once, on a connection `reuse` allows, and answers its head. It
/// follows no redirect, resends nothing, decodes no content coding and sets
/// no timeout of its own: the driver does each. An origin that answered
/// [`TransportFailure::Http11Required`] is sent HTTP/1.1 from then on.
pub trait Transport: Send + Sync {
    /// Sends `hop` on a connection `reuse` allows and answers its head.
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent>;
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
    /// The host's name resolved only to addresses of the family the
    /// Session holds no local address of (ADR 0044):
    /// `address_family_unavailable` (`net::ERR_ADDRESS_UNREACHABLE`).
    AddressFamily {
        /// The host's name.
        name: String,
        /// The one family the Session holds.
        held: IpFamily,
    },
    /// The host refused the connection (`net::ERR_CONNECTION_REFUSED`).
    ConnectionRefused,
    /// The connection timed out (`net::ERR_CONNECTION_TIMED_OUT`).
    ConnectionTimedOut,
    /// The connection was reset (`net::ERR_CONNECTION_RESET`).
    ConnectionReset,
    /// The connection closed before the answer was whole
    /// (`net::ERR_CONNECTION_CLOSED`).
    ConnectionClosed,
    /// An idle connection an earlier hop left closed or was reset before
    /// any byte of the answer: the site never processed the request
    /// (`net::ERR_EMPTY_RESPONSE`), which only a read is sent again for.
    ClosedAtReuse,
    /// The site refused the HTTP/2 stream unprocessed: `REFUSED_STREAM`, or
    /// a `GOAWAY` whose last stream is below it
    /// (`net::ERR_HTTP2_SERVER_REFUSED_STREAM`).
    RefusedStream,
    /// The site asked over HTTP/2 for HTTP/1.1 (`HTTP_1_1_REQUIRED`), having
    /// processed nothing (`net::ERR_HTTP_1_1_REQUIRED`).
    Http11Required,
    /// The site reset the HTTP/2 stream, or closed the connection, with
    /// `ENHANCE_YOUR_CALM`: it asks for less load, and its domain backs off
    /// as on a 503 (ADR 0118). Chromium names it as every HTTP/2 reset it
    /// has no error of its own for (`net::ERR_HTTP2_PROTOCOL_ERROR`).
    EnhanceYourCalm,
    /// The host's certificate failed verification so, and no exception
    /// covers its domain (`net::ERR_CERT_AUTHORITY_INVALID`): nothing was
    /// sent to it, and the attempt ends with `certificate_invalid` (ADR
    /// 0042).
    CertificateInvalid(CertificateFailure),
    /// The host's pinned key changed, now or earlier in the Run, and
    /// nothing was sent to it (`net::ERR_SSL_PINNED_KEY_NOT_IN_CERT_CHAIN`):
    /// `certificate_changed` (ADR 0042).
    CertificateChanged(CertificateChanged),
    /// The client certificate `tls.client_certificates` lists for the host
    /// could not be presented, its key's reference unresolved, the host
    /// reached through an exception or the certificate unusable
    /// (`net::ERR_BAD_SSL_CLIENT_AUTH_CERT`): the error it ends with.
    ClientCertificate(CallFailure),
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
    /// The browser of the browser-fetch tier ended the hop with this
    /// error of its own, its network error named as the browser names it
    /// (ADR 0020): the browser resends and decodes itself, so it is never
    /// sent again.
    Browser(BrowserFailure),
    /// Any other failure: the generic case (ADR 0118; `net::ERR_FAILED`).
    Other,
}

/// An error a transport ended a hop with, two of them equal when their
/// codes and messages are.
#[derive(Clone)]
pub struct CallFailure(pub CallError);

impl PartialEq for CallFailure {
    fn eq(&self, other: &Self) -> bool {
        self.0.code() == other.0.code() && self.0.message() == other.0.message()
    }
}

impl Eq for CallFailure {}

impl std::fmt::Debug for CallFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallFailure")
            .field("code", &self.0.code())
            .field("message", &self.0.message())
            .finish()
    }
}

/// The error the browser of the browser-fetch tier ended a hop with, two
/// of them equal when their codes and messages are.
#[derive(Clone)]
pub struct BrowserFailure(pub CallError);

impl PartialEq for BrowserFailure {
    fn eq(&self, other: &Self) -> bool {
        self.0.code() == other.0.code() && self.0.message() == other.0.message()
    }
}

impl Eq for BrowserFailure {}

impl std::fmt::Debug for BrowserFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserFailure")
            .field("code", &self.0.code())
            .field("message", &self.0.message())
            .finish()
    }
}

/// What an HTTP tier's hop met, as the backoff classes read it (spec
/// #579): its answer's status and `Proxy-Status`, the proxy having
/// forwarded the hop itself when `forwarded` (an `http:` URL sent to an
/// HTTP proxy), so that only then can the field name the proxy's own error.
pub fn met(status: u16, headers: &[(String, String)], forwarded: bool) -> Met<'_> {
    Met::Response {
        status,
        proxy_status: headers
            .iter()
            .find(|(name, _)| name == "proxy-status")
            .map(|(_, value)| value.as_str()),
        forwarded,
    }
}

impl TransportFailure {
    /// The failure as a hop to `url` reports it: an egress refusal names
    /// the hop's URL, whatever URL the library's error carried; any other
    /// failure stays as it is. Every transport's translation ends here
    /// (ADR 0129).
    pub(crate) fn of_hop(self, url: &Url) -> Self {
        match self {
            Self::Egress(refused) => Self::Egress(Refused {
                url: url.to_string(),
                ..refused
            }),
            failure => failure,
        }
    }

    /// What a hop that failed so met, as the backoff classes read it: its
    /// network error, Chrome's name for it, the proxy's own for a tunnel
    /// the proxy refused, asked credentials for or never opened.
    pub fn met(&self) -> Met<'static> {
        Met::Failure {
            error: self.network_error(),
        }
    }

    /// The network error Chrome names the same failure with.
    pub fn network_error(&self) -> &'static str {
        match self {
            Self::Egress(_) => "net::ERR_BLOCKED_BY_CLIENT",
            Self::NameNotResolved => "net::ERR_NAME_NOT_RESOLVED",
            Self::AddressFamily { .. } => "net::ERR_ADDRESS_UNREACHABLE",
            Self::ConnectionRefused => "net::ERR_CONNECTION_REFUSED",
            Self::ConnectionTimedOut => "net::ERR_CONNECTION_TIMED_OUT",
            Self::ConnectionReset => "net::ERR_CONNECTION_RESET",
            Self::ConnectionClosed => "net::ERR_CONNECTION_CLOSED",
            Self::ClosedAtReuse => "net::ERR_EMPTY_RESPONSE",
            Self::RefusedStream => "net::ERR_HTTP2_SERVER_REFUSED_STREAM",
            Self::Http11Required => "net::ERR_HTTP_1_1_REQUIRED",
            Self::EnhanceYourCalm => "net::ERR_HTTP2_PROTOCOL_ERROR",
            Self::CertificateInvalid(_) => "net::ERR_CERT_AUTHORITY_INVALID",
            Self::CertificateChanged(_) => "net::ERR_SSL_PINNED_KEY_NOT_IN_CERT_CHAIN",
            Self::ClientCertificate(_) => "net::ERR_BAD_SSL_CLIENT_AUTH_CERT",
            Self::Tls => "net::ERR_SSL_PROTOCOL_ERROR",
            Self::InvalidResponse => "net::ERR_INVALID_HTTP_RESPONSE",
            Self::TunnelRefused => "net::ERR_TUNNEL_CONNECTION_FAILED",
            Self::ProxyAuthenticationRequired => "net::ERR_PROXY_AUTH_REQUESTED",
            Self::ProxyUnreachable => "net::ERR_PROXY_CONNECTION_FAILED",
            Self::ContentDecodingFailed => "net::ERR_CONTENT_DECODING_FAILED",
            Self::Browser(_) | Self::Other => "net::ERR_FAILED",
        }
    }

    /// What it ends the attempt fetching `url` with: `egress_refused` for
    /// the egress policy's refusal, the browser's own error on the
    /// browser-fetch tier, `certificate_invalid` and `certificate_changed`
    /// for a certificate (ADR 0042), neither retryable, the client
    /// certificate's own error, and `navigation_failed` with its network
    /// error for every other failure, retryable (ADR 0118).
    pub fn error(&self, url: &str) -> CallError {
        match self {
            Self::Egress(refused) => return refused.error(),
            Self::Browser(failure) => return failure.0.clone(),
            Self::ClientCertificate(failure) => return failure.0.clone(),
            Self::CertificateChanged(changed) => return changed.error(),
            Self::CertificateInvalid(failure) => {
                let host = Url::parse(url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_default();
                return CertificateInvalid {
                    host,
                    failure: failure.clone(),
                }
                .error();
            }
            Self::AddressFamily { name, held } => {
                return FamilyUnavailable {
                    name: name.clone(),
                    held: *held,
                }
                .into();
            }
            _ => {}
        }
        let error = self.network_error();
        CallError::new(
            &NAVIGATION_FAILED,
            format!("The fetch of {url} failed: {error}."),
            &NavigationFailedDetails {
                url: Some(url.to_owned()),
                error: Some(error.to_owned()),
            },
        )
    }

    /// Whether the site never processed a hop that failed so, which is
    /// then sent once more on a new connection (ADR 0118): a refused
    /// HTTP/2 stream and `HTTP_1_1_REQUIRED` whatever the hop, and a reused
    /// connection's close only for a [read](Hop::read) (RFC 9110, section
    /// 9.2.2).
    fn unprocessed(&self, hop: &Hop) -> bool {
        match self {
            Self::RefusedStream | Self::Http11Required => true,
            Self::ClosedAtReuse => hop.read,
            _ => false,
        }
    }
}

/// The head of `hop`'s answer through `transport`, sent once more on a new
/// connection when the site never processed it (ADR 0118): a failure
/// [`TransportFailure::unprocessed`] names, a 421, or a 408 a reused
/// connection gave a read. The resend's answer or failure is the hop's.
async fn answer(
    transport: &dyn Transport,
    hop: &Hop,
    reuse: Reuse,
) -> Result<Sent, TransportFailure> {
    match transport.send(hop, reuse).await {
        Err(failure) if !failure.unprocessed(hop) => Err(failure),
        Ok(sent) if sent.status != 421 && !(sent.status == 408 && sent.reused && hop.read) => {
            Ok(sent)
        }
        Err(_) | Ok(_) => transport.send(hop, Reuse::Never).await,
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
    /// Its body, decoded as its `Content-Encoding` lists; empty when it
    /// was dropped for its type.
    pub body: Vec<u8>,
    /// The type of a response that would reach the handler but whose
    /// type the request's Label does not take (`crawl.content_types`,
    /// ADR 0119), its body then dropped: unread when its head named the
    /// type, read and sniffed when it named none.
    pub unaccepted: Option<String>,
    /// The invalid-certificate exception its host was reached through (ADR
    /// 0042, pitfall H03), which the Provenance of its Records marks.
    pub certificate_exception: Option<ExceptionUsed>,
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

/// The codings a fetch asks for, as Chrome asks for them.
const ACCEPT_ENCODING: &str = "gzip, deflate, br, zstd";

/// Fetches `hop` through `transport` under `deadlines` on `clock`: its head
/// within `timeout.first_byte`, its whole body within `timeout.navigation`,
/// then its content codings decoded. A hop that names no
/// `Accept-Encoding` asks for every coding Chrome decodes.
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
    fetch_on(transport, hop, Reuse::Allowed, deadlines, clock, None).await
}

/// [`fetch`] of a crawl request's hop judged by `accepts`, its Label's
/// `crawl.content_types` (ADR 0119): a response that would reach the
/// handler with a type it does not take answers with its body dropped and
/// its type in [`Fetched::unaccepted`], unread once its head named the
/// type.
///
/// # Errors
///
/// As [`fetch`]'s.
///
/// # Cancel safety
///
/// As [`fetch`]'s.
pub(crate) async fn fetch_accepting(
    transport: &dyn Transport,
    hop: &Hop,
    deadlines: &Deadlines,
    clock: &Clock,
    accepts: Option<&Accepts>,
) -> Result<Fetched, CallError> {
    fetch_on(transport, hop, Reuse::Allowed, deadlines, clock, accepts).await
}

/// [`fetch`] on a new connection, never one an earlier hop left: a
/// hedge's (ADR 0040).
///
/// # Errors
///
/// As [`fetch`]'s.
///
/// # Cancel safety
///
/// As [`fetch`]'s.
pub(crate) async fn fetch_anew(
    transport: &dyn Transport,
    hop: &Hop,
    deadlines: &Deadlines,
    clock: &Clock,
    accepts: Option<&Accepts>,
) -> Result<Fetched, CallError> {
    fetch_on(transport, hop, Reuse::Never, deadlines, clock, accepts).await
}

/// [`fetch`] whose first send goes on a connection `reuse` allows, judged
/// by `accepts` as [`fetch_accepting`] says.
async fn fetch_on(
    transport: &dyn Transport,
    hop: &Hop,
    reuse: Reuse,
    deadlines: &Deadlines,
    clock: &Clock,
    accepts: Option<&Accepts>,
) -> Result<Fetched, CallError> {
    let url = hop.url.as_str();
    let asked;
    let hop = if hop
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("accept-encoding"))
    {
        hop
    } else {
        let mut with_codings = hop.clone();
        with_codings
            .headers
            .push(("accept-encoding".to_owned(), ACCEPT_ENCODING.to_owned()));
        asked = with_codings;
        &asked
    };
    let whole = async {
        let mut sent = deadlines
            .first_byte
            .within(clock, "first_byte", answer(transport, hop, reuse))
            .await?
            .map_err(|failure| failure.error(url))?;
        let declared = sent
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.clone());
        // A type the head names is judged before any byte of the body is
        // read: the body is dropped unread, its connection with it.
        if let (Some(accepts), Some(declared)) = (accepts, &declared)
            && let Some(refused) = accepts.refused(sent.status, &typed(Some(declared), &[], "").0)
        {
            return Ok(Fetched {
                url: hop.url.clone(),
                status: sent.status,
                headers: sent.headers,
                body: Vec::new(),
                unaccepted: Some(refused),
                certificate_exception: sent.certificate_exception,
            });
        }
        let expected = content_length(&sent.headers);
        let mut body = Vec::new();
        // Any failure once the head arrived cuts the body (ADR 0118).
        while let Some(chunk) = sent
            .body
            .chunk()
            .await
            .map_err(|_| truncated(url, body.len(), expected))?
        {
            body.extend_from_slice(&chunk);
        }
        let codings = sent
            .headers
            .iter()
            .filter(|(name, _)| name == "content-encoding" && !sent.decoded)
            .map(|(_, value)| value.as_str());
        let received = body.len();
        let body = decoding::decode(codings, body).map_err(|failed| match failed {
            DecodingFailed::Cut => truncated(url, received, expected),
            DecodingFailed::Corrupt => TransportFailure::ContentDecodingFailed.error(url),
        })?;
        if decoding::still_compressed(declared.as_deref(), &body) {
            return Err(TransportFailure::ContentDecodingFailed.error(url));
        }
        // With no type in its head, the body is sniffed as Chromium sniffs
        // it, so it had to be read.
        let unaccepted = match (accepts, &declared) {
            (Some(accepts), None) => {
                accepts.refused(sent.status, &typed(None, &body, hop.url.path()).0)
            }
            _ => None,
        };
        Ok::<Fetched, CallError>(Fetched {
            url: hop.url.clone(),
            status: sent.status,
            headers: sent.headers,
            body: if unaccepted.is_some() {
                Vec::new()
            } else {
                body
            },
            unaccepted,
            certificate_exception: sent.certificate_exception,
        })
    };
    deadlines
        .navigation
        .within(clock, "navigation", whole)
        .await?
}

/// The length an answer's `Content-Length` declares, if it declares one.
fn content_length(headers: &[(String, String)]) -> Option<u64> {
    headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.trim().parse().ok())
}

/// `response_truncated` for the body of `url`, which broke off after
/// `received` bytes, as sent, of the `expected` its `Content-Length`
/// declared (ADR 0118): retryable, and never handed to a handler.
fn truncated(url: &str, received: usize, expected: Option<u64>) -> CallError {
    let received = u64::try_from(received).unwrap_or(u64::MAX);
    let of = expected.map_or_else(String::new, |expected| format!(" of {expected}"));
    CallError::new(
        &RESPONSE_TRUNCATED,
        format!("The body of {url} broke off after {received} bytes{of}."),
        &ResponseTruncatedDetails {
            url: url.to_owned(),
            received,
            expected,
        },
    )
}

/// Judges a hop to `url` under the Session's egress policy `egress` before
/// anything is sent (ADR 0017): its URL, and the address the URL names in
/// place of a host, which no resolver answers: a transport connects to it
/// without a lookup, and a proxy, which would connect to it, is never sent
/// it.
///
/// # Errors
///
/// [`Refused`] by the domain policy, or by the address check for an address
/// a page may not reach.
pub(crate) fn judged(url: &Url, egress: &Policy) -> Result<(), Refused> {
    let request = egress::Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    if let Judgment::Refused(refused) = egress::judge(&request, egress) {
        return Err(refused);
    }
    let address = match url.host() {
        Some(Host::Ipv4(address)) => IpAddr::V4(address),
        Some(Host::Ipv6(address)) => IpAddr::V6(address),
        _ => return Ok(()),
    };
    let port = url.port_or_known_default().unwrap_or_default();
    egress::judge_addresses(url.as_str(), &[SocketAddr::new(address, port)], egress)
}
