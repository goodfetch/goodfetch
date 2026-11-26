//! The `mailgun` provider: one call to Mailgun's
//! `POST /v3/<domain>/messages` per delivery, in the region its sending
//! domain lives in, the subject and both parts sent as a form,
//! authenticated as `api` with the API key. Mailgun's errors carry no
//! code, so its translation point maps each answer by its status to the
//! driver's [`Failure`].

use data_encoding::BASE64;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use goodfetch_protocol::notifications::MailgunRegion;
use hyper::StatusCode;
use hyper::body::Bytes;
use hyper::header::AUTHORIZATION;
use secrecy::{ExposeSecret as _, SecretString};
use url::{Url, form_urlencoded};

use super::Address;
use super::api::{self, Api, ApiFailure, ApiUnusable, Known, Table};
use super::render::Parts;
use crate::failure::Failure;

/// Mailgun's translation table.
pub(super) const TABLE: Table = Table {
    party: "Mailgun",
    kind: |_, _| None,
    wait: retry_after::wait_of,
    known,
};

/// A `mailgun` provider: the API key, the sending domain, and how the
/// channel reaches Mailgun.
#[derive(Debug)]
pub struct Mailgun {
    api: Api,
}

impl Mailgun {
    /// Mailgun at `base` when given, else at `region`'s API,
    /// `https://api.mailgun.net` or `https://api.eu.mailgun.net`, sending
    /// from `domain` with `api_key`, reached through `endpoint`. A `base`
    /// is where a stand-in answers in Mailgun's place; a Run gives none.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable`] when no HTTP header may carry `api_key`, `domain`
    /// is empty or holds anything but ASCII letters, digits, `-` and `.`,
    /// or `base` is no `http` or `https` URL.
    pub fn new(
        api_key: &SecretString,
        domain: &str,
        region: MailgunRegion,
        base: Option<Url>,
        endpoint: NotificationEndpoint,
    ) -> Result<Self, ApiUnusable> {
        let usable =
            |character: char| character.is_ascii_alphanumeric() || matches!(character, '-' | '.');
        if domain.is_empty() || !domain.chars().all(usable) {
            return Err(ApiUnusable::Domain);
        }
        let default = match region {
            MailgunRegion::Us => "https://api.mailgun.net",
            MailgunRegion::Eu => "https://api.eu.mailgun.net",
        };
        let mut url = api::url(base, default, "/v3/")?;
        url.path_segments_mut()
            .map_err(|()| ApiUnusable::Url)?
            .pop_if_empty()
            .push(domain)
            .push("messages");
        let login = BASE64.encode(format!("api:{}", api_key.expose_secret()).as_bytes());
        let credential = api::sensitive(&format!("Basic {login}"))?;
        Ok(Self {
            api: Api::new(
                &url,
                "application/x-www-form-urlencoded",
                (AUTHORIZATION, credential),
                endpoint,
            )?,
        })
    }

    /// Mailgun's host.
    pub(super) fn host(&self) -> &str {
        self.api.host()
    }

    /// Sends `parts` from `from` to each of `to`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; Mailgun may or may not
    /// have taken the message.
    pub(super) async fn send(
        &self,
        from: &Address,
        to: &[Address],
        parts: &Parts,
        clock: Clock,
    ) -> Result<(), ApiFailure> {
        self.api
            .post(None, Bytes::from(form(from, to, parts)), clock, &TABLE)
            .await
    }
}

/// The form Mailgun's `messages` takes: `from`, a `to` for each
/// recipient, `subject`, `text` and `html`.
fn form(from: &Address, to: &[Address], parts: &Parts) -> String {
    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("from", from.as_str());
    for address in to {
        form.append_pair("to", address.as_str());
    }
    form.append_pair("subject", &parts.subject)
        .append_pair("text", &parts.text)
        .append_pair("html", &parts.html)
        .finish()
}

/// The failure a Mailgun answer is, by its status, if the table knows it.
fn known(status: StatusCode, _: Option<&str>) -> Option<Known> {
    match status {
        // A bad request, a domain Mailgun does not know in this region, or
        // a message over its size.
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::PAYLOAD_TOO_LARGE => {
            Some(Known::Is(Failure::Rejected))
        }
        // Mailgun's own rate-limit status, beside `429`.
        status if status.as_u16() == 420 => Some(Known::RateLimited),
        status => api::by_status(status),
    }
}
