//! What the email providers reached over an HTTP API share (issue 1389):
//! one `POST` through the Notification channel endpoint per delivery, an
//! answer that is no success read into an [`ApiFailure`], and the
//! translation point (ADR 0127) each provider fills with its own [`Table`]
//! of the codes and statuses it answers with.

use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use hyper::body::Bytes;
use hyper::header::{CONTENT_TYPE, HeaderName, HeaderValue, USER_AGENT};
use hyper::{HeaderMap, Method, Request, StatusCode, Uri};
use url::Url;

use super::PARTY_TEXT_LIMIT;
use crate::failure::{ChannelText, Failure};

/// The `User-Agent` every provider's request carries: Resend refuses a
/// request without one.
const AGENT: &str = "goodfetch-notification";

/// The longest error code or name an [`ApiFailure`] keeps: every provider's
/// is a few dozen characters at most.
const KIND_LIMIT: usize = 64;

/// Why an HTTP API provider cannot be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApiUnusable {
    /// The API key holds what no HTTP header may carry.
    #[error("an email provider's API key is text an HTTP header can carry")]
    Key,
    /// The API's URL is no absolute `http` or `https` URL with a host.
    #[error("an email provider's API URL is an absolute `http` or `https` URL")]
    Url,
    /// The Mailgun domain is empty or holds anything but letters, digits,
    /// `-` and `.`.
    #[error("a Mailgun domain is letters, digits, `-` and `.`, such as `mg.example.com`")]
    Domain,
}

/// Why a provider reached over its HTTP API did not take the message.
#[derive(Debug)]
#[non_exhaustive]
pub enum ApiFailure {
    /// The endpoint refused the provider, failed its certificate, or gave
    /// no whole answer.
    Endpoint(WholeFailure),
    /// An answer that is no success.
    Status {
        /// Its status.
        status: StatusCode,
        /// The error code or name the provider gave, as its table reads it,
        /// when it is one: bounded letters, digits, `_`, `-` and `.`.
        kind: Option<String>,
        /// The wait the answer asked for.
        retry_after: Option<Duration>,
        /// The start of its body.
        text: ChannelText,
    },
}

/// A provider's translation table: its name in the logs, how it names an
/// error and the wait it asks for, and the failure each answer it knows
/// is.
pub(super) struct Table {
    /// The provider's name, such as `Resend`.
    pub(super) party: &'static str,
    /// The error code or name an answer gives in its headers or body.
    pub(super) kind: fn(&HeaderMap, &[u8]) -> Option<String>,
    /// The wait an answer received at the given time asks for.
    pub(super) wait: fn(&HeaderMap, SystemTime) -> Option<Duration>,
    /// The failure an answer of this status and error code or name is, if
    /// the table knows it.
    pub(super) known: fn(StatusCode, Option<&str>) -> Option<Known>,
}

/// A failure a table knows: rate limited, with the answer's own wait, or
/// any other.
pub(super) enum Known {
    RateLimited,
    Is(Failure),
}

/// The failure an answer's status is when the table knows nothing more
/// specific, as every provider's status codes share it.
pub(super) fn by_status(status: StatusCode) -> Option<Known> {
    match status {
        StatusCode::TOO_MANY_REQUESTS => Some(Known::RateLimited),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Some(Known::Is(Failure::Unauthorized)),
        StatusCode::REQUEST_TIMEOUT => Some(Known::Is(Failure::Timeout)),
        status if status.is_server_error() => Some(Known::Is(Failure::ServerError)),
        _ => None,
    }
}

/// The URL of `path` at `base` when given, else at the provider's own API
/// `default`. A `base` is where a stand-in answers in a provider's place,
/// as the tests' stand-ins do; the channel's settings give none, so a Run
/// reaches the provider itself.
///
/// # Errors
///
/// [`ApiUnusable::Url`] when `path` cannot join the base.
pub(super) fn url(base: Option<Url>, default: &str, path: &str) -> Result<Url, ApiUnusable> {
    base.map_or_else(|| Url::parse(default), Ok)
        .and_then(|base| base.join(path))
        .map_err(|_| ApiUnusable::Url)
}

/// `value` as a header value that is never logged.
///
/// # Errors
///
/// [`ApiUnusable::Key`] when no HTTP header may carry it.
pub(super) fn sensitive(value: &str) -> Result<HeaderValue, ApiUnusable> {
    let mut header = HeaderValue::from_str(value).map_err(|_| ApiUnusable::Key)?;
    header.set_sensitive(true);
    Ok(header)
}

/// A provider's API: where its requests go, the headers each carries, and
/// how the channel reaches it.
#[derive(Debug)]
pub(super) struct Api {
    uri: Uri,
    host: String,
    headers: HeaderMap,
    endpoint: NotificationEndpoint,
}

impl Api {
    /// Requests to `url` of `content_type`, each carrying `credential`,
    /// reached through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable::Url`] when `url` is no `http` or `https` URL with a
    /// host.
    pub(super) fn new(
        url: &Url,
        content_type: &'static str,
        credential: (HeaderName, HeaderValue),
        endpoint: NotificationEndpoint,
    ) -> Result<Self, ApiUnusable> {
        let host = match (url.scheme(), url.host_str()) {
            ("http" | "https", Some(host)) => host.to_owned(),
            _ => return Err(ApiUnusable::Url),
        };
        let uri = Uri::try_from(url.as_str()).map_err(|_| ApiUnusable::Url)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        headers.insert(USER_AGENT, HeaderValue::from_static(AGENT));
        headers.insert(credential.0, credential.1);
        Ok(Self {
            uri,
            host,
            headers,
            endpoint,
        })
    }

    /// The API's host.
    pub(super) fn host(&self) -> &str {
        &self.host
    }

    /// Posts `body` with the API's headers and `extra`, reading an answer
    /// that is no success by `table`, received at `clock`'s time.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; the provider may or may
    /// not have taken the message.
    pub(super) async fn post(
        &self,
        extra: Option<(HeaderName, HeaderValue)>,
        body: Bytes,
        clock: Clock,
        table: &Table,
    ) -> Result<(), ApiFailure> {
        let mut request = Request::new(body);
        *request.method_mut() = Method::POST;
        *request.uri_mut() = self.uri.clone();
        *request.headers_mut() = self.headers.clone();
        if let Some((name, value)) = extra {
            request.headers_mut().insert(name, value);
        }
        send(&self.endpoint, request, clock, table).await
    }
}

/// Sends `request` through `endpoint`, reading an answer that is no
/// success by `table`, received at `clock`'s time.
///
/// # Cancel safety
///
/// Dropping the future closes the connection; the provider may or may not
/// have taken the message.
pub(super) async fn send(
    endpoint: &NotificationEndpoint,
    request: Request<Bytes>,
    clock: Clock,
    table: &Table,
) -> Result<(), ApiFailure> {
    let answer = match endpoint.send(request).await {
        Ok(answer) => answer,
        // An answer over the endpoint's limit still has its status: a
        // success took the message, whatever followed it.
        Err(WholeFailure::TooLong { status }) if status.is_success() => return Ok(()),
        Err(WholeFailure::TooLong { status }) => {
            return Err(ApiFailure::Status {
                status,
                kind: None,
                retry_after: None,
                text: ChannelText::new(String::new()),
            });
        }
        Err(failure) => return Err(ApiFailure::Endpoint(failure)),
    };
    let status = answer.status();
    if status.is_success() {
        return Ok(());
    }
    let body = answer.body();
    let start = body.get(..PARTY_TEXT_LIMIT).unwrap_or(body);
    Err(ApiFailure::Status {
        status,
        kind: (table.kind)(answer.headers(), body).filter(|kind| is_code(kind)),
        retry_after: (table.wait)(answer.headers(), clock.wall()),
        text: ChannelText::new(String::from_utf8_lossy(start)),
    })
}

/// Whether `kind`, read from a provider's answer, reads as an error code or
/// name: at most [`KIND_LIMIT`] ASCII letters, digits, `_`, `-` and `.`.
/// Anything else is no code a table knows, and would put unbounded party
/// text in a log field (STYLE.md, Logging and secrets).
fn is_code(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= KIND_LIMIT
        && kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// A provider's translation point (ADR 0127): the driver's failure for each
/// answer its `table` knows, and [`Failure::Unrecognized`], logged at
/// `warn` with its text, for any other.
pub(super) fn translate(failure: ApiFailure, table: &Table) -> Failure {
    match failure {
        ApiFailure::Endpoint(
            WholeFailure::CertificateInvalid(_)
            | WholeFailure::ClientCertificate(_)
            | WholeFailure::Network,
        ) => Failure::Unreachable,
        ApiFailure::Endpoint(WholeFailure::Refused(_)) => Failure::Rejected,
        ApiFailure::Endpoint(other) => {
            ChannelText::new(format!("{other:?}")).unrecognized(&format!(
                "{}'s endpoint failed in a way its translation table does not know",
                table.party
            ))
        }
        ApiFailure::Status {
            status,
            kind,
            retry_after,
            text,
        } => match (table.known)(status, kind.as_deref()) {
            Some(Known::RateLimited) => Failure::RateLimited { retry_after },
            Some(Known::Is(failure)) => failure,
            None => {
                tracing::warn!(
                    party = table.party,
                    status = status.as_u16(),
                    kind,
                    "an email provider answered with an error its translation table does not know"
                );
                text.unrecognized(&format!("the start of {}'s answer", table.party))
            }
        },
    }
}
