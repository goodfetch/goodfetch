//! The `webhook` channel (ADR 0027; issue 1385): a Notification posted as a
//! Standard Webhooks delivery, `{ type, timestamp, format, data }`, whose
//! `webhook-id` is the event ID, signed by `goodfetch_engine`'s
//! `webhook_signing` with every key of the channel's `webhook.secret`. It
//! posts through the engine's Notification webhook endpoint, its own HTTP
//! client and never a Session's (ADR 0088), so no cookie goes with it, and
//! its translation point maps the receiver's answer to the driver's
//! [`Failure`](crate::failure::Failure).

use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::{Clock, rfc3339};
use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use goodfetch_engine::secrets::PartyText;
use goodfetch_engine::webhook_signing::{self, Signer};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::RunId;
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode, Uri};
use serde::Serialize;
use url::Url;

use crate::driver::Channel;
use crate::failure::Failure;
use crate::message::Message;

/// The `major.minor` version of the delivery's body, a contract of its own
/// (ADR 0030), in its `format` field.
pub const FORMAT: &str = "0.1";

/// How long a delivery waits for the receiver's whole answer before it
/// counts as [`Failure::Timeout`].
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// The most bytes of a receiver's answer an unknown answer keeps as its
/// party text.
const PARTY_TEXT_LIMIT: usize = 1024;

/// What a delivery's `data` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// The whole Notification: its IDs, event, Product, title, text and
    /// link.
    Full,
    /// Its IDs and link only, which Integrations' triggers fetch the rest
    /// from (ADR 0027).
    Thin,
}

/// A `webhook` channel.
#[derive(Debug)]
pub struct Webhook {
    name: String,
    url: Url,
    uri: Uri,
    signer: Signer,
    payload: Payload,
    endpoint: NotificationEndpoint,
    clock: Clock,
}

/// Why a URL cannot be a webhook's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a webhook's URL names a host, such as `https://hooks.example.com/goodfetch`")]
pub struct UrlUnusable;

/// Why a delivery got no success from the receiver.
#[derive(Debug)]
#[non_exhaustive]
pub enum WebhookFailure {
    /// The endpoint refused it, failed its certificate, or gave no whole
    /// answer.
    Endpoint(WholeFailure),
    /// No whole answer within [`SEND_TIMEOUT`].
    TimedOut,
    /// An answer other than a success.
    Status {
        /// Its status.
        status: StatusCode,
        /// The wait its `Retry-After` asks for, if it gave one.
        retry_after: Option<Duration>,
        /// The start of its body, kept as party text.
        text: String,
    },
}

impl Webhook {
    /// The channel `name` posting to `url`, signed by `signer`, with
    /// `payload`, through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`UrlUnusable`] when `url` names no host.
    pub fn new(
        name: impl Into<String>,
        url: Url,
        signer: Signer,
        payload: Payload,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, UrlUnusable> {
        if url.host_str().is_none_or(str::is_empty) {
            return Err(UrlUnusable);
        }
        let uri = Uri::try_from(url.as_str()).map_err(|_| UrlUnusable)?;
        Ok(Self {
            name: name.into(),
            url,
            uri,
            signer,
            payload,
            endpoint,
            clock,
        })
    }

    /// The signed request that delivers `message`.
    fn request(&self, message: &Message) -> Request<Bytes> {
        let body = body(message, self.payload);
        let timestamp = self
            .clock
            .wall()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        let headers = self
            .signer
            .headers(
                &webhook_signing::Message {
                    id: message.event_id.as_str(),
                    timestamp,
                    body: &body,
                },
                &[("content-type", "application/json")],
            )
            .expect("invariant: an event ID is non-empty ASCII with no `.`, CR, LF or NUL");
        headers
            .iter()
            .fold(
                Request::builder()
                    .method(Method::POST)
                    .uri(self.uri.clone()),
                |request, (name, value)| request.header(name, value),
            )
            .body(Bytes::from(body))
            .expect("invariant: the URI parsed and every header was checked")
    }
}

impl Channel for Webhook {
    type Failure = WebhookFailure;

    fn name(&self) -> &str {
        &self.name
    }

    fn destination(&self) -> &str {
        self.url.host_str().unwrap_or_default()
    }

    async fn send(&self, message: &Message) -> Result<(), WebhookFailure> {
        let answer = tokio::select! {
            answer = self.endpoint.send(self.request(message)) => answer.map_err(WebhookFailure::Endpoint)?,
            () = self.clock.sleep(SEND_TIMEOUT) => return Err(WebhookFailure::TimedOut),
        };
        let status = answer.status();
        if status.is_success() {
            return Ok(());
        }
        let body = answer.body();
        let start = body.get(..PARTY_TEXT_LIMIT).unwrap_or(body);
        Err(WebhookFailure::Status {
            status,
            retry_after: retry_after::wait_of(answer.headers(), self.clock.wall()),
            text: String::from_utf8_lossy(start).into_owned(),
        })
    }

    fn map_failure(&self, failure: WebhookFailure) -> Failure {
        translate(failure)
    }
}

/// The webhook's translation point (ADR 0127): the driver's failure for
/// each answer it knows, and [`Failure::Unrecognized`], logged at `warn`
/// with its status alone, for any other.
fn translate(failure: WebhookFailure) -> Failure {
    match failure {
        WebhookFailure::Endpoint(WholeFailure::CertificateInvalid(_) | WholeFailure::Network) => {
            Failure::Unreachable
        }
        WebhookFailure::TimedOut
        | WebhookFailure::Status {
            status: StatusCode::REQUEST_TIMEOUT,
            ..
        } => Failure::Timeout,
        WebhookFailure::Status {
            status: StatusCode::TOO_MANY_REQUESTS,
            retry_after,
            ..
        } => Failure::RateLimited { retry_after },
        WebhookFailure::Status { status, .. } if status.is_server_error() => Failure::ServerError,
        WebhookFailure::Status {
            status: StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN,
            ..
        } => Failure::Unauthorized,
        WebhookFailure::Status {
            status: StatusCode::NOT_FOUND | StatusCode::GONE,
            ..
        } => Failure::Gone,
        WebhookFailure::Endpoint(WholeFailure::Refused(_))
        | WebhookFailure::Status {
            status:
                StatusCode::BAD_REQUEST
                | StatusCode::PAYLOAD_TOO_LARGE
                | StatusCode::UNSUPPORTED_MEDIA_TYPE
                | StatusCode::UNPROCESSABLE_ENTITY,
            ..
        } => Failure::Rejected,
        WebhookFailure::Status { status, text, .. } => {
            tracing::warn!(
                status = status.as_u16(),
                "a webhook answered with a status its translation table does not know"
            );
            Failure::Unrecognized(PartyText::new(format!("{status}: {text}")))
        }
        WebhookFailure::Endpoint(other) => {
            tracing::warn!(
                failure = ?other,
                "a webhook's endpoint failed in a way its translation table does not know"
            );
            Failure::Unrecognized(PartyText::new(format!("{other:?}")))
        }
    }
}

/// A delivery's body: the Standard Webhooks envelope around `message`'s
/// `payload`.
fn body(message: &Message, payload: Payload) -> Vec<u8> {
    let run_id = message.run_id.as_ref().map(RunId::as_str);
    let schedule_id = message.schedule_id.as_deref();
    let link = message.link.as_ref().map(Url::as_str);
    let event_id = message.event_id.as_str();
    let envelope = |data| Delivery {
        kind: format!("notification.{}", message.event),
        timestamp: rfc3339(message.occurred_at),
        format: FORMAT,
        data,
    };
    match payload {
        Payload::Full => serde_json::to_vec(&envelope(Data::Full(Full {
            event_id,
            event: &message.event,
            run_id,
            schedule_id,
            product: message.product,
            title: &message.title,
            text: &message.text,
            link,
        }))),
        Payload::Thin => serde_json::to_vec(&envelope(Data::Thin(Thin {
            event_id,
            run_id,
            schedule_id,
            link,
        }))),
    }
    .expect("invariant: a delivery's body is strings and a Product, which always serialize")
}

/// The Standard Webhooks envelope.
#[derive(Serialize)]
struct Delivery<'a> {
    #[serde(rename = "type")]
    kind: String,
    timestamp: String,
    format: &'static str,
    data: Data<'a>,
}

/// A delivery's `data`.
#[derive(Serialize)]
#[serde(untagged)]
enum Data<'a> {
    Full(Full<'a>),
    Thin(Thin<'a>),
}

/// The whole Notification.
#[derive(Serialize)]
struct Full<'a> {
    event_id: &'a str,
    event: &'a str,
    run_id: Option<&'a str>,
    schedule_id: Option<&'a str>,
    product: Product,
    title: &'a str,
    text: &'a str,
    link: Option<&'a str>,
}

/// The thin payload: IDs and link only.
#[derive(Serialize)]
struct Thin<'a> {
    event_id: &'a str,
    run_id: Option<&'a str>,
    schedule_id: Option<&'a str>,
    link: Option<&'a str>,
}
