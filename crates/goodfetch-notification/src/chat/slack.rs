//! The `slack` channel's platform: an incoming webhook's message of
//! blocks, a header with the title, a section with the text, and a context
//! line with the Product, the event and the link, over a top-level `text`
//! that notifications and screen readers show. Slack answers a failure as
//! a plain-text error name, such as `channel_not_found`, which its table
//! reads first, then the status.

use hyper::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use super::context;
use crate::api::{self, Known, Table};
use crate::failure::Failure;
use crate::message::{Message, clipped};

/// The longest `header` block's text Slack takes, in characters.
const HEADER_LIMIT: usize = 150;

/// The longest `section` block's text Slack takes, in characters.
const SECTION_LIMIT: usize = 3000;

/// Slack's translation table.
pub(super) const TABLE: Table = Table {
    party: "Slack",
    kind: error_name,
    wait: api::header_wait,
    known,
};

/// The message that reports `message` on Slack.
pub(super) fn body(message: &Message) -> Value {
    let title = if message.title.trim().is_empty() {
        message.event.to_string()
    } else {
        message.title.clone()
    };
    let mut blocks = vec![json!({
        "type": "header",
        "text": { "type": "plain_text", "text": clipped(&title, HEADER_LIMIT) },
    })];
    if !message.text.trim().is_empty() {
        blocks.push(json!({
            "type": "section",
            "text": { "type": "plain_text", "text": clipped(&message.text, SECTION_LIMIT) },
        }));
    }
    let mut elements = vec![json!({ "type": "plain_text", "text": context(message) })];
    if let Some(link) = message.link_without_credentials() {
        elements.push(json!({
            "type": "mrkdwn",
            "text": format!("<{}|Open>", link_text(link.as_str())),
        }));
    }
    blocks.push(json!({ "type": "context", "elements": elements }));
    json!({ "text": escaped(&title), "blocks": blocks })
}

/// `text` as Slack's `mrkdwn` takes it: its three control characters
/// escaped, so no text reads as a mention or a link.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `url` as a `mrkdwn` link's target holds it: escaped, its `|`, which
/// would end the target, percent-encoded.
fn link_text(url: &str) -> String {
    escaped(url).replace('|', "%7C")
}

/// The error name Slack's plain-text body gives, such as
/// `channel_not_found`.
fn error_name(_: &HeaderMap, body: &[u8]) -> Option<String> {
    Some(std::str::from_utf8(body).ok()?.trim().to_owned())
}

/// The failure a Slack answer is, by its error name and else by its status,
/// if the table knows it: the names of Slack's incoming webhook errors.
fn known(status: StatusCode, name: Option<&str>) -> Option<Known> {
    name.and_then(by_name)
        .map(Known::Is)
        .or_else(|| api::by_status(status))
}

/// The failure an error name Slack gives is, if the table knows it.
fn by_name(name: &str) -> Option<Failure> {
    match name {
        "invalid_payload"
        | "no_text"
        | "too_many_attachments"
        | "posting_to_general_channel_denied" => Some(Failure::Rejected),
        "channel_not_found"
        | "channel_is_archived"
        | "user_not_found"
        | "no_service"
        | "no_service_id"
        | "no_team"
        | "team_disabled" => Some(Failure::Gone),
        "invalid_token" | "action_prohibited" => Some(Failure::Unauthorized),
        "rollup_error" => Some(Failure::ServerError),
        _ => None,
    }
}
