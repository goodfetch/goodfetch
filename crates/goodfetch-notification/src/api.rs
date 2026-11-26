//! What the channels reached over an HTTP API share (issues 1387 and
//! 1389): the email providers', and the chat platforms'. One `POST`
//! through the Notification channel endpoint per delivery, an answer that
//! is no success read into an [`ApiFailure`], and the translation point
//! (ADR 0127) each provider or platform fills with its own [`Table`] of the
//! codes and statuses it answers with. A destination the egress policy
//! refuses fails the channel's configuration, before any delivery.

use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use goodfetch_protocol::egress::EgressRule;
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HeaderName, HeaderValue, USER_AGENT};
use hyper::{HeaderMap, Method, Request, StatusCode, Uri};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::failure::{ChannelText, Failure};

/// The most bytes of a party's answer an unknown answer keeps as its party
/// text.
pub(crate) const PARTY_TEXT_LIMIT: usize = 1024;

/// The `User-Agent` every request carries: Resend refuses a request
/// without one.
const AGENT: &str = "goodfetch-notification";

/// The longest error code or name an [`ApiFailure`] keeps: every
/// provider's and platform's is a few dozen characters at most.
const KIND_LIMIT: usize = 64;

/// Why a channel reached over an HTTP API cannot be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApiUnusable {
    /// The API key, or bot token, holds what no request may carry.
    #[error("a provider's API key is text an HTTP header can carry")]
    Key,
    /// The API's URL is no absolute `http` or `https` URL with a host.
    #[error("a provider's API URL is an absolute `http` or `https` URL")]
    Url,
    /// The Mailgun domain is empty or holds anything but letters, digits,
    /// `-` and `.`.
    #[error("a Mailgun domain is letters, digits, `-` and `.`, such as `mg.example.com`")]
    Domain,
    /// The egress policy refuses the destination by this rule.
    #[error("the egress policy's `{}` rule refuses the destination (ADR 0017)", .0.as_str())]
    Refused(EgressRule),
    /// A `teams` URL is an Office 365 connector's, which takes no Adaptive
    /// Card and which Microsoft retired.
    #[error("a `teams` URL is a Workflows webhook's, not an Office 365 connector's")]
    TeamsConnector,
    /// A Telegram chat is neither a number nor a public chat's `@username`.
    #[error("a Telegram chat is a number, such as `-1001234567890`, or an `@username`")]
    ChatId,
    /// An AWS region is empty or holds anything but lower-case letters,
    /// digits and `-`.
    #[error("an AWS region is lower-case letters, digits and `-`, such as `eu-west-1`")]
    Region,
    /// An SNS topic's ARN is no SNS topic's.
    #[error(
        "an SNS topic ARN is `arn:<partition>:sns:<region>:<account>:<topic>`, its topic letters, digits, `-` and `_`, a FIFO topic's ending in `.fifo`"
    )]
    Topic,
}

/// Why a provider or platform reached over its HTTP API did not take the
/// message.
#[derive(Debug)]
#[non_exhaustive]
pub enum ApiFailure {
    /// The endpoint refused the destination, failed its certificate, or
    /// gave no whole answer.
    Endpoint(WholeFailure),
    /// An answer that is no success.
    Status {
        /// Its status.
        status: StatusCode,
        /// The error code or name the party gave, as its table reads it,
        /// when it is one: bounded letters, digits, `_`, `-` and `.`.
        kind: Option<String>,
        /// The wait the answer asked for.
        retry_after: Option<Duration>,
        /// The start of its body.
        text: ChannelText,
    },
}

/// A party's translation table: its name in the logs, how it names an
/// error and the wait it asks for, and the failure each answer it knows
/// is.
pub(crate) struct Table {
    /// The party's name, such as `Resend`.
    pub(crate) party: &'static str,
    /// The error code or name an answer gives in its headers or body.
    pub(crate) kind: fn(&HeaderMap, &[u8]) -> Option<String>,
    /// The wait an answer, its headers and body, received at the given time
    /// asks for.
    pub(crate) wait: fn(&HeaderMap, &[u8], SystemTime) -> Option<Duration>,
    /// The failure an answer of this status and error code or name is, if
    /// the table knows it.
    pub(crate) known: fn(StatusCode, Option<&str>) -> Option<Known>,
}

/// A failure a table knows: rate limited, with the answer's own wait, or
/// any other.
pub(crate) enum Known {
    RateLimited,
    Is(Failure),
}

/// The wait an answer's `Retry-After` asks for, as most parties give it.
pub(crate) fn header_wait(
    headers: &HeaderMap,
    _body: &[u8],
    received: SystemTime,
) -> Option<Duration> {
    retry_after::wait_of(headers, received)
}

/// The failure an answer's status is when the table knows nothing more
/// specific, as every party's status codes share it.
pub(crate) fn by_status(status: StatusCode) -> Option<Known> {
    match status {
        StatusCode::TOO_MANY_REQUESTS => Some(Known::RateLimited),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Some(Known::Is(Failure::Unauthorized)),
        StatusCode::REQUEST_TIMEOUT => Some(Known::Is(Failure::Timeout)),
        status if status.is_server_error() => Some(Known::Is(Failure::ServerError)),
        _ => None,
    }
}

/// Whether `region` reads as an AWS region, such as `eu-west-1`: lower-case
/// letters, digits and `-`, as the regional endpoint's host holds it.
pub(crate) fn is_aws_region(region: &str) -> bool {
    !region.is_empty()
        && region.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

/// The host of `url`, a party's endpoint, once it is an `http` or `https`
/// URL with a host the egress policy takes before any lookup (ADR 0017).
///
/// # Errors
///
/// [`ApiUnusable::Url`] for no such URL, and [`ApiUnusable::Refused`] when
/// the egress policy refuses it.
pub(crate) fn usable_host<'a>(
    url: &'a Url,
    endpoint: &NotificationEndpoint,
) -> Result<&'a str, ApiUnusable> {
    let ("http" | "https", Some(host)) = (url.scheme(), url.host_str()) else {
        return Err(ApiUnusable::Url);
    };
    endpoint
        .judge(url)
        .map_err(|refused| ApiUnusable::Refused(refused.rule))?;
    Ok(host)
}

/// The URL of `path` at `base` when given, else at the party's own API
/// `default`. A `base` is where a stand-in answers in a party's place, as
/// the tests' stand-ins do; the channel's settings give none, so a Run
/// reaches the party itself.
///
/// # Errors
///
/// [`ApiUnusable::Url`] when `path` cannot join the base.
pub(crate) fn url(base: Option<Url>, default: &str, path: &str) -> Result<Url, ApiUnusable> {
    base.map_or_else(|| Url::parse(default), Ok)
        .and_then(|base| base.join(path))
        .map_err(|_| ApiUnusable::Url)
}

/// `value` as a header value that is never logged.
///
/// # Errors
///
/// [`ApiUnusable::Key`] when no HTTP header may carry it.
pub(crate) fn sensitive(value: &str) -> Result<HeaderValue, ApiUnusable> {
    let mut header = HeaderValue::from_str(value).map_err(|_| ApiUnusable::Key)?;
    header.set_sensitive(true);
    Ok(header)
}

/// A party's API: where its requests go, the headers each carries, and how
/// the channel reaches it. Its URL may carry the destination's credential,
/// a webhook's secret path or a bot's token, so its `Debug` names the host
/// alone.
pub(crate) struct Api {
    uri: Uri,
    host: String,
    headers: HeaderMap,
    endpoint: NotificationEndpoint,
}

impl Api {
    /// Requests to `url` of `content_type`, each carrying `credential` when
    /// given, reached through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable::Url`] when `url` is no `http` or `https` URL with a
    /// host, and [`ApiUnusable::Refused`] when the egress policy refuses it
    /// before any lookup (ADR 0017).
    pub(crate) fn new(
        url: &Url,
        content_type: &'static str,
        credential: Option<(HeaderName, HeaderValue)>,
        endpoint: NotificationEndpoint,
    ) -> Result<Self, ApiUnusable> {
        let host = usable_host(url, &endpoint)?.to_owned();
        let uri = Uri::try_from(url.as_str()).map_err(|_| ApiUnusable::Url)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        headers.insert(USER_AGENT, HeaderValue::from_static(AGENT));
        if let Some((name, value)) = credential {
            headers.insert(name, value);
        }
        Ok(Self {
            uri,
            host,
            headers,
            endpoint,
        })
    }

    /// JSON requests to `path` at `base` when given, else at the party's
    /// own API `default`, each carrying `api_key` as a bearer token,
    /// reached through `endpoint`: what Resend's and SendGrid's APIs take.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable::Key`] when no HTTP header may carry `api_key`, and
    /// as [`url`] and [`Api::new`].
    pub(crate) fn bearer_json(
        api_key: &SecretString,
        base: Option<Url>,
        default: &str,
        path: &str,
        endpoint: NotificationEndpoint,
    ) -> Result<Self, ApiUnusable> {
        let url = url(base, default, path)?;
        let credential = sensitive(&format!("Bearer {}", api_key.expose_secret()))?;
        Self::new(
            &url,
            "application/json",
            Some((AUTHORIZATION, credential)),
            endpoint,
        )
    }

    /// The API's host.
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    /// Posts `body` with the API's headers and `extra`, reading an answer
    /// that is no success by `table`, received at `clock`'s time.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; the party may or may not
    /// have taken the message.
    pub(crate) async fn post(
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

impl std::fmt::Debug for Api {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Api")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// Sends `request` through `endpoint`, reading an answer that is no
/// success by `table`, received at `clock`'s time.
///
/// # Cancel safety
///
/// Dropping the future closes the connection; the party may or may not
/// have taken the message.
pub(crate) async fn send(
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
        retry_after: (table.wait)(answer.headers(), body, clock.wall()),
        text: ChannelText::new(String::from_utf8_lossy(start)),
    })
}

/// Whether `kind`, read from a party's answer, reads as an error code or
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

/// A party's translation point (ADR 0127): the driver's failure for each
/// answer its `table` knows, and [`Failure::Unrecognized`], logged at
/// `warn` with its text, for any other.
pub(crate) fn translate(failure: ApiFailure, table: &Table) -> Failure {
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
                    "a provider answered with an error its translation table does not know"
                );
                text.unrecognized(&format!("the start of {}'s answer", table.party))
            }
        },
    }
}
