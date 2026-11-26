//! The `resend` provider: one call to Resend's `POST /emails` per
//! delivery, the subject and both parts sent as JSON, with an
//! `Idempotency-Key` (ADR 0027) that is the same on every attempt of one
//! Notification on one channel, so a retry Resend already took sends no
//! second email. Its translation point maps each answer, by the error
//! `name` Resend gives in its body, else by its status, to the driver's
//! [`Failure`].

use data_encoding::HEXLOWER;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, HeaderName, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::Address;
use super::api::{self, Api, ApiFailure, ApiUnusable, Known, Table};
use super::render::Parts;
use crate::failure::Failure;
use crate::message::EventId;

/// Resend's API.
const BASE: &str = "https://api.resend.com";

/// The path of its `Send Email`.
const SEND_PATH: &str = "/emails";

/// The header its idempotency key goes in.
const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

/// How many hex digits of the channel name's SHA-256 the key keeps: with
/// an event ID of at most 128 characters, the key stays within Resend's
/// 256.
const CHANNEL_DIGITS: usize = 32;

/// Resend's translation table.
pub(super) const TABLE: Table = Table {
    party: "Resend",
    kind: error_name,
    wait: retry_after::wait_of,
    known,
};

/// A `resend` provider: the API key, and how the channel reaches Resend.
#[derive(Debug)]
pub struct Resend {
    api: Api,
}

impl Resend {
    /// Resend at `base` when given, else at `https://api.resend.com`, sent
    /// `api_key`, reached through `endpoint`. A `base` is where a stand-in
    /// answers in Resend's place; a Run gives none.
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

    /// Resend's host.
    pub(super) fn host(&self) -> &str {
        self.api.host()
    }

    /// Sends `parts` from `from` to each of `to`, under the idempotency key
    /// of `event_id` on the channel `channel`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; Resend may or may not
    /// have taken the message, and a retry under the same key sends no
    /// second one.
    pub(super) async fn send(
        &self,
        from: &Address,
        to: &[Address],
        parts: &Parts,
        channel: &str,
        event_id: &EventId,
        clock: Clock,
    ) -> Result<(), ApiFailure> {
        let body = json!({
            "from": from.as_str(),
            "to": to.iter().map(Address::as_str).collect::<Vec<_>>(),
            "subject": parts.subject,
            "text": parts.text,
            "html": parts.html,
        });
        self.api
            .post(
                Some((IDEMPOTENCY_KEY, idempotency_key(channel, event_id))),
                Bytes::from(body.to_string()),
                clock,
                &TABLE,
            )
            .await
    }
}

/// The idempotency key of `event_id` on the channel `channel`: the event ID
/// and the first hex digits of the channel name's SHA-256, so two channels
/// sending one Notification through one Resend account never share a key.
fn idempotency_key(channel: &str, event_id: &EventId) -> HeaderValue {
    let digest = HEXLOWER.encode(&Sha256::digest(channel.as_bytes()));
    let digits = digest.get(..CHANNEL_DIGITS).unwrap_or(&digest);
    HeaderValue::from_str(&format!("{}.{digits}", event_id.as_str()))
        .expect("invariant: an event ID and hex digits are ASCII letters, digits, `_`, `-` and `.`")
}

/// The error `name` Resend's body gives, such as `validation_error`.
fn error_name(_: &HeaderMap, body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    Some(body.get("name")?.as_str()?.to_owned())
}

/// The failure a Resend answer is, by its error name and else by its
/// status, if the table knows it.
fn known(status: StatusCode, name: Option<&str>) -> Option<Known> {
    name.and_then(|name| by_name(status, name))
        .or_else(|| api::by_status(status))
}

/// The failure an error name Resend gives with `status` is, if the table
/// knows it: the names of Resend's error reference.
fn by_name(status: StatusCode, name: &str) -> Option<Known> {
    match name {
        "rate_limit_exceeded"
        | "daily_quota_exceeded"
        | "monthly_quota_exceeded"
        | "concurrent_idempotent_requests" => Some(Known::RateLimited),
        "missing_api_key" | "invalid_api_key" | "restricted_api_key" | "suspended_api_key"
        | "invalid_permission" => Some(Known::Is(Failure::Unauthorized)),
        // Resend answers an unverified domain or a key it refuses with a
        // `validation_error` of 401 or 403.
        "validation_error"
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) =>
        {
            Some(Known::Is(Failure::Unauthorized))
        }
        "validation_error"
        | "missing_required_field"
        | "missing_required_parameter"
        | "invalid_parameter"
        | "invalid_attachment"
        | "invalid_idempotency_key"
        | "invalid_idempotent_request"
        | "not_found"
        | "method_not_allowed"
        | "return_path_unavailable"
        | "forge_unavailable"
        | "forge_not_enabled" => Some(Known::Is(Failure::Rejected)),
        // A `resource_locked` is another operation holding the resource for
        // now, which passes.
        "application_error" | "service_unavailable" | "dns_lookup_failed" | "resource_locked" => {
            Some(Known::Is(Failure::ServerError))
        }
        _ => None,
    }
}
