//! The `ses` provider: one Amazon SES v2 `SendEmail` call per delivery,
//! the rendered message sent raw, the call signed with Signature Version 4
//! for the `ses` service in the channel's region. Its translation point
//! maps each answer, by the error type SES names in its
//! `x-amzn-ErrorType` header or its body, else by its status, to the
//! driver's [`Failure`].

use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_engine::aws_signing::{self, Credentials, Scope, Unsignable};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use hyper::body::Bytes;
use hyper::{HeaderMap, StatusCode};
use serde_json::{Value, json};
use url::Url;

use super::{Address, PARTY_TEXT_LIMIT};
use crate::failure::{ChannelText, Failure};

/// The path of SES v2's `SendEmail`.
const SEND_EMAIL_PATH: &str = "/v2/email/outbound-emails";

/// The service name SES requests are signed for.
const SERVICE: &str = "ses";

/// An `ses` provider: the region, the access key, and how the channel
/// reaches SES.
#[derive(Debug)]
pub struct Ses {
    region: String,
    url: Url,
    credentials: Credentials,
    endpoint: NotificationEndpoint,
}

/// Why a text is no AWS region the channel sends through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("an SES region is lower-case letters, digits and `-`, such as `eu-west-1`")]
#[non_exhaustive]
pub struct RegionUnusable;

/// Why SES did not take the message.
#[derive(Debug)]
#[non_exhaustive]
pub enum SesFailure {
    /// The endpoint refused SES, failed its certificate, or gave no whole
    /// answer.
    Endpoint(WholeFailure),
    /// The access key cannot sign a request, so none was sent.
    Unsignable(Unsignable),
    /// An answer that is no success.
    Status {
        /// Its status.
        status: StatusCode,
        /// The error type SES named, such as `MessageRejected`, without
        /// its `Exception` suffix.
        kind: Option<String>,
        /// The wait its `Retry-After` asked for.
        retry_after: Option<Duration>,
        /// The start of its body.
        text: ChannelText,
    },
}

impl Ses {
    /// SES in `region`, at `endpoint_url` when given, else at its regional
    /// endpoint, `https://email.<region>.amazonaws.com`, signed by
    /// `credentials` and reached through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`RegionUnusable`] when `region` is empty or holds anything but
    /// lower-case letters, digits and `-`.
    pub fn new(
        region: &str,
        endpoint_url: Option<Url>,
        credentials: Credentials,
        endpoint: NotificationEndpoint,
    ) -> Result<Self, RegionUnusable> {
        let usable = |character: char| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        };
        if region.is_empty() || !region.chars().all(usable) {
            return Err(RegionUnusable);
        }
        let base = match endpoint_url {
            Some(url) => url,
            None => Url::parse(&format!("https://email.{region}.amazonaws.com"))
                .map_err(|_| RegionUnusable)?,
        };
        let url = base.join(SEND_EMAIL_PATH).map_err(|_| RegionUnusable)?;
        Ok(Self {
            region: region.to_owned(),
            url,
            credentials,
            endpoint,
        })
    }

    /// SES's host.
    pub(super) fn host(&self) -> &str {
        self.url.host_str().unwrap_or_default()
    }

    /// Sends the rendered message `raw` from `from` to each of `to`, signed
    /// at `clock`'s time.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; SES may or may not have
    /// taken the message.
    pub(super) async fn send(
        &self,
        from: &Address,
        to: &[Address],
        raw: Vec<u8>,
        clock: Clock,
    ) -> Result<(), SesFailure> {
        let body = json!({
            "FromEmailAddress": from.as_str(),
            "Destination": {
                "ToAddresses": to.iter().map(Address::as_str).collect::<Vec<_>>(),
            },
            "Content": { "Raw": { "Data": BASE64.encode(&raw) } },
        });
        let request = aws_signing::signed_post(
            &self.url,
            &[("content-type", "application/json")],
            Bytes::from(body.to_string()),
            Scope {
                region: &self.region,
                service: SERVICE,
            },
            &self.credentials,
            clock.wall(),
        )
        .map_err(SesFailure::Unsignable)?;
        let answer = match self.endpoint.send(request).await {
            Ok(answer) => answer,
            // An answer over the endpoint's limit still has its status: a
            // success took the message, whatever followed it.
            Err(WholeFailure::TooLong { status }) if status.is_success() => return Ok(()),
            Err(WholeFailure::TooLong { status }) => {
                return Err(SesFailure::Status {
                    status,
                    kind: None,
                    retry_after: None,
                    text: ChannelText::new(String::new()),
                });
            }
            Err(failure) => return Err(SesFailure::Endpoint(failure)),
        };
        let status = answer.status();
        if status.is_success() {
            return Ok(());
        }
        let body = answer.body();
        let start = body.get(..PARTY_TEXT_LIMIT).unwrap_or(body);
        Err(SesFailure::Status {
            status,
            kind: error_type(answer.headers(), body),
            retry_after: retry_after::wait_of(answer.headers(), clock.wall()),
            text: ChannelText::new(String::from_utf8_lossy(start)),
        })
    }
}

/// The error type an answer names: in its `x-amzn-ErrorType` header,
/// before any `:`, else in its body's `__type` or `code`, after any `#`;
/// without its `Exception` suffix.
fn error_type(headers: &HeaderMap, body: &[u8]) -> Option<String> {
    let named = headers
        .get("x-amzn-errortype")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(':').next().unwrap_or(value).to_owned())
        .or_else(|| {
            let body: Value = serde_json::from_slice(body).ok()?;
            let named = body.get("__type").or_else(|| body.get("code"))?.as_str()?;
            Some(named.rsplit('#').next().unwrap_or(named).to_owned())
        })?;
    let named = named.trim();
    Some(named.strip_suffix("Exception").unwrap_or(named).to_owned())
}

/// The SES translation point (ADR 0127): the driver's failure for each
/// answer it knows, by error type and else by status, and
/// [`Failure::Unrecognized`], logged at `warn` with its text, for any
/// other.
pub(super) fn translate(failure: SesFailure) -> Failure {
    match failure {
        SesFailure::Endpoint(
            WholeFailure::CertificateInvalid(_)
            | WholeFailure::ClientCertificate(_)
            | WholeFailure::Network,
        ) => Failure::Unreachable,
        SesFailure::Endpoint(WholeFailure::Refused(_)) => Failure::Rejected,
        SesFailure::Endpoint(other) => ChannelText::new(format!("{other:?}"))
            .unrecognized("SES's endpoint failed in a way its translation table does not know"),
        SesFailure::Unsignable(_) => Failure::Unauthorized,
        SesFailure::Status {
            status,
            kind,
            retry_after,
            text,
        } => match kind
            .as_deref()
            .and_then(by_type)
            .or_else(|| by_status(status))
        {
            Some(Known::RateLimited) => Failure::RateLimited { retry_after },
            Some(Known::Is(failure)) => failure,
            None => {
                tracing::warn!(
                    status = status.as_u16(),
                    kind,
                    "SES answered with an error its translation table does not know"
                );
                text.unrecognized("the start of SES's answer")
            }
        },
    }
}

/// A failure the SES table knows: rate limited, with the answer's own
/// wait, or any other.
enum Known {
    RateLimited,
    Is(Failure),
}

/// The failure an error type SES names is, if the table knows it.
fn by_type(kind: &str) -> Option<Known> {
    match kind {
        "TooManyRequests" | "LimitExceeded" | "Throttling" => Some(Known::RateLimited),
        "AccessDenied"
        | "UnrecognizedClient"
        | "InvalidSignature"
        | "SignatureDoesNotMatch"
        | "IncompleteSignature"
        | "InvalidClientTokenId"
        | "MissingAuthenticationToken"
        | "ExpiredToken"
        | "AccountSuspended"
        | "SendingPaused" => Some(Known::Is(Failure::Unauthorized)),
        "MessageRejected" | "MailFromDomainNotVerified" | "BadRequest" | "NotFound" => {
            Some(Known::Is(Failure::Rejected))
        }
        _ => None,
    }
}

/// The failure an answer's status is when its error type is unknown, if
/// the table knows the status.
fn by_status(status: StatusCode) -> Option<Known> {
    match status {
        StatusCode::TOO_MANY_REQUESTS => Some(Known::RateLimited),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Some(Known::Is(Failure::Unauthorized)),
        StatusCode::REQUEST_TIMEOUT => Some(Known::Is(Failure::Timeout)),
        status if status.is_server_error() => Some(Known::Is(Failure::ServerError)),
        _ => None,
    }
}
