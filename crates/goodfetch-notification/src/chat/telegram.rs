//! The `telegram` channel's platform: a Telegram bot's `sendMessage` to
//! one chat, in Telegram's HTML: the title in bold, the text, the context
//! in italics and an `Open` link, with no link preview. The bot's token is in the API's path, so the
//! channel's URL is a secret like a webhook's. Telegram answers a failure
//! as `{"error_code": …, "parameters": …}`: a group moved to a supergroup
//! gives the new chat's ID, and a rate limit its `retry_after` in seconds.

use std::fmt::Write as _;
use std::time::{Duration, SystemTime};

use hyper::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use url::Url;

use super::context;
use crate::api::{self, ApiUnusable, Known, Table};
use crate::failure::Failure;
use crate::message::{Message, clipped, html_escaped};

/// Telegram's Bot API.
const BASE: &str = "https://api.telegram.org";

/// The longest title the message is given, in characters.
const TITLE_LIMIT: usize = 256;

/// The longest text the message is given, in characters: with the title,
/// the context and the link's word, within the 4,096 Telegram takes once
/// its HTML is read.
const TEXT_LIMIT: usize = 3500;

/// The longest context line the message is given, in characters.
const CONTEXT_LIMIT: usize = 256;

/// The longest `@username` of a public chat, in characters.
const USERNAME_LIMIT: usize = 32;

/// The shortest `@username` of a public chat, in characters.
const USERNAME_MIN: usize = 5;

/// The kind a table gives a failure that names the chat a group moved to.
const MIGRATED: &str = "migrated";

/// Telegram's translation table.
pub(super) const TABLE: Table = Table {
    party: "Telegram",
    kind: error_kind,
    wait,
    known,
};

/// Whether `chat_id` names a chat as Telegram's `chat_id` takes it: a
/// number, negative for a group, or a public chat's `@username`.
pub(super) fn is_chat_id(chat_id: &str) -> bool {
    if let Some(username) = chat_id.strip_prefix('@') {
        return (USERNAME_MIN..=USERNAME_LIMIT).contains(&username.len())
            && username
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    }
    let digits = chat_id.strip_prefix('-').unwrap_or(chat_id);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// The URL of `bot_token`'s `sendMessage` at `base` when given, else at
/// Telegram's Bot API.
///
/// # Errors
///
/// [`ApiUnusable::Key`] for a token that is no bot's, `123456:ABC-…`, and
/// [`ApiUnusable::Url`] when its path cannot join the base.
pub(super) fn url(base: Option<Url>, bot_token: &SecretString) -> Result<Url, ApiUnusable> {
    let token = bot_token.expose_secret();
    let is_token = token.contains(':')
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'));
    if !is_token {
        return Err(ApiUnusable::Key);
    }
    api::url(base, BASE, &format!("/bot{token}/sendMessage"))
}

/// The message that reports `message` to the chat `chat_id`.
pub(super) fn body(chat_id: &str, message: &Message) -> Value {
    let mut text = format!(
        "<b>{}</b>",
        html_escaped(&clipped(&message.title, TITLE_LIMIT))
    );
    if !message.text.trim().is_empty() {
        text.push_str("\n\n");
        text.push_str(&html_escaped(&clipped(&message.text, TEXT_LIMIT)));
    }
    let _ = write!(
        text,
        "\n\n<i>{}</i>",
        html_escaped(&clipped(&context(message), CONTEXT_LIMIT))
    );
    if let Some(link) = message.link_without_credentials() {
        let _ = write!(
            text,
            "\n\n<a href=\"{}\">Open</a>",
            html_escaped(link.as_str())
        );
    }
    json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
        "link_preview_options": { "is_disabled": true },
    })
}

/// The kind of a Telegram failure: [`MIGRATED`] when its `parameters`
/// name the chat a group moved to, else its `error_code`.
fn error_kind(_: &HeaderMap, body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    if body
        .get("parameters")
        .and_then(|parameters| parameters.get("migrate_to_chat_id"))
        .is_some()
    {
        return Some(MIGRATED.to_owned());
    }
    Some(body.get("error_code")?.as_u64()?.to_string())
}

/// The wait a Telegram answer asks for: the `retry_after` of its
/// `parameters`, in seconds, else its `Retry-After`.
fn wait(headers: &HeaderMap, body: &[u8], received: SystemTime) -> Option<Duration> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| body.get("parameters")?.get("retry_after")?.as_u64())
        .map(Duration::from_secs)
        .or_else(|| api::header_wait(headers, body, received))
}

/// The failure a Telegram answer is, if the table knows it: a group that
/// moved is gone, for the channel must name the new chat; then by status,
/// as Telegram's Bot API answers its errors.
fn known(status: StatusCode, kind: Option<&str>) -> Option<Known> {
    if kind == Some(MIGRATED) {
        return Some(Known::Is(Failure::Gone));
    }
    match status {
        // A token Telegram does not know is a 404 of its API's path.
        StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND => Some(Known::Is(Failure::Unauthorized)),
        // The bot was blocked, or removed from the chat.
        StatusCode::FORBIDDEN => Some(Known::Is(Failure::Gone)),
        StatusCode::BAD_REQUEST => Some(Known::Is(Failure::Rejected)),
        status => api::by_status(status),
    }
}
