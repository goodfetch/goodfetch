//! The `postmark` provider: one call to Postmark's `POST /email` per
//! delivery, the subject and both parts sent as JSON on the `outbound`
//! message stream, authenticated by the server token. Its translation
//! point maps each answer, by the `ErrorCode` Postmark gives in its body,
//! else by its status, to the driver's [`Failure`].

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use hyper::body::Bytes;
use hyper::header::{ACCEPT, HeaderName, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use url::Url;

use super::Address;
use super::api::{self, Api, ApiFailure, ApiUnusable, Known, Table};
use super::render::Parts;
use crate::failure::Failure;

/// Postmark's API.
const BASE: &str = "https://api.postmarkapp.com";

/// The path of its single-email send.
const SEND_PATH: &str = "/email";

/// The header its server token goes in.
const SERVER_TOKEN: HeaderName = HeaderName::from_static("x-postmark-server-token");

/// Postmark's translation table.
pub(super) const TABLE: Table = Table {
    party: "Postmark",
    kind: error_code,
    wait: retry_after::wait_of,
    known,
};

/// A `postmark` provider: the server token, and how the channel reaches
/// Postmark.
#[derive(Debug)]
pub struct Postmark {
    api: Api,
}

impl Postmark {
    /// Postmark at `base` when given, else at
    /// `https://api.postmarkapp.com`, sent the server token `api_key`,
    /// reached through `endpoint`. A `base` is where a stand-in answers in
    /// Postmark's place; a Run gives none.
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
        let credential = api::sensitive(api_key.expose_secret())?;
        Ok(Self {
            api: Api::new(
                &url,
                "application/json",
                (SERVER_TOKEN, credential),
                endpoint,
            )?,
        })
    }

    /// Postmark's host.
    pub(super) fn host(&self) -> &str {
        self.api.host()
    }

    /// Sends `parts` from `from` to each of `to`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; Postmark may or may not
    /// have taken the message.
    pub(super) async fn send(
        &self,
        from: &Address,
        to: &[Address],
        parts: &Parts,
        clock: Clock,
    ) -> Result<(), ApiFailure> {
        // No address holds a `,`, so the joined list splits back into the
        // same addresses.
        let to = to
            .iter()
            .map(Address::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let body = json!({
            "From": from.as_str(),
            "To": to,
            "Subject": parts.subject,
            "TextBody": parts.text,
            "HtmlBody": parts.html,
            "MessageStream": "outbound",
        });
        self.api
            .post(
                Some((ACCEPT, HeaderValue::from_static("application/json"))),
                Bytes::from(body.to_string()),
                clock,
                &TABLE,
            )
            .await
    }
}

/// The `ErrorCode` Postmark's body gives, such as `406`.
fn error_code(_: &HeaderMap, body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    Some(body.get("ErrorCode")?.as_u64()?.to_string())
}

/// The failure a Postmark answer is, by its error code and else by its
/// status, if the table knows it.
fn known(status: StatusCode, code: Option<&str>) -> Option<Known> {
    code.and_then(by_code).or_else(|| match status {
        StatusCode::PAYLOAD_TOO_LARGE | StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            Some(Known::Is(Failure::Rejected))
        }
        status => api::by_status(status),
    })
}

/// The failure an error code Postmark gives is, if the table knows it.
fn by_code(code: &str) -> Option<Known> {
    match code {
        // The server token is missing or wrong, or the account may not
        // send yet.
        "10" | "412" | "413" => Some(Known::Is(Failure::Unauthorized)),
        // The recipient is inactive: it bounced, complained or
        // unsubscribed.
        "406" => Some(Known::Is(Failure::Gone)),
        // The request, its sender or its stream is invalid.
        "300" | "402" | "403" | "422" | "1235" | "1236" | "1480" => {
            Some(Known::Is(Failure::Rejected))
        }
        "100" | "101" => Some(Known::Is(Failure::ServerError)),
        _ => None,
    }
}
