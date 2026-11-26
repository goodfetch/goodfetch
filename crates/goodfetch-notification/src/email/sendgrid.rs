//! The `sendgrid` provider: one call to SendGrid's `POST /v3/mail/send`
//! per delivery, the subject and both parts sent as JSON to one
//! personalization of every recipient. SendGrid's errors carry no code, so
//! its translation point maps each answer by its status to the driver's
//! [`Failure`], a rate limit waiting until its `X-RateLimit-Reset`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use hyper::body::Bytes;
use hyper::header::AUTHORIZATION;
use hyper::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use url::Url;

use super::Address;
use super::api::{self, Api, ApiFailure, ApiUnusable, Known, Table};
use super::render::Parts;
use crate::failure::Failure;

/// SendGrid's API.
const BASE: &str = "https://api.sendgrid.com";

/// The path of its Mail Send.
const SEND_PATH: &str = "/v3/mail/send";

/// SendGrid's translation table.
pub(super) const TABLE: Table = Table {
    party: "SendGrid",
    kind: |_, _| None,
    wait,
    known,
};

/// A `sendgrid` provider: the API key, and how the channel reaches
/// SendGrid.
#[derive(Debug)]
pub struct Sendgrid {
    api: Api,
}

impl Sendgrid {
    /// SendGrid at `base` when given, else at `https://api.sendgrid.com`,
    /// sent `api_key`, reached through `endpoint`. A `base` is where a
    /// stand-in answers in SendGrid's place; a Run gives none.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable`] when no HTTP header may carry `api_key`, or `base`
    /// is no `http` or `https` URL.
    pub fn new(
        api_key: &SecretString,
        base: Option<Url>,
        endpoint: NotificationEndpoint,
    ) -> Result<Self, ApiUnusable> {
        let url = api::url(base, BASE, SEND_PATH)?;
        let credential = api::sensitive(&format!("Bearer {}", api_key.expose_secret()))?;
        Ok(Self {
            api: Api::new(
                &url,
                "application/json",
                (AUTHORIZATION, credential),
                endpoint,
            )?,
        })
    }

    /// SendGrid's host.
    pub(super) fn host(&self) -> &str {
        self.api.host()
    }

    /// Sends `parts` from `from` to each of `to`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; SendGrid may or may not
    /// have taken the message.
    pub(super) async fn send(
        &self,
        from: &Address,
        to: &[Address],
        parts: &Parts,
        clock: Clock,
    ) -> Result<(), ApiFailure> {
        let to = to
            .iter()
            .map(|address| json!({ "email": address.as_str() }))
            .collect::<Vec<_>>();
        let body = json!({
            "personalizations": [{ "to": to }],
            "from": { "email": from.as_str() },
            "subject": parts.subject,
            "content": [
                { "type": "text/plain", "value": parts.text },
                { "type": "text/html", "value": parts.html },
            ],
        });
        self.api
            .post(None, Bytes::from(body.to_string()), clock, &TABLE)
            .await
    }
}

/// The wait a SendGrid answer received at `received` asks for: until its
/// `X-RateLimit-Reset`, in seconds since the epoch, else its
/// `Retry-After`.
fn wait(headers: &HeaderMap, received: SystemTime) -> Option<Duration> {
    let reset = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)));
    match reset {
        Some(reset) => Some(reset.duration_since(received).unwrap_or_default()),
        None => retry_after::wait_of(headers, received),
    }
}

/// The failure a SendGrid answer is, by its status, if the table knows
/// it.
fn known(status: StatusCode, _: Option<&str>) -> Option<Known> {
    match status {
        StatusCode::BAD_REQUEST
        | StatusCode::NOT_FOUND
        | StatusCode::METHOD_NOT_ALLOWED
        | StatusCode::NOT_ACCEPTABLE
        | StatusCode::PAYLOAD_TOO_LARGE => Some(Known::Is(Failure::Rejected)),
        status => api::by_status(status),
    }
}
