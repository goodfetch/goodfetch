//! The `discord` channel's platform: a Discord webhook's message, one
//! embed with the title, the text, the link, the time of the event and a
//! footer with the Product and the event, which mentions no one. Discord
//! answers a failure as `{"code": …}`, a number its table reads first,
//! then the status, and a rate limit with `retry_after` in seconds.

use std::time::{Duration, SystemTime};

use goodfetch_protocol::point_in_time;
use hyper::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use super::context;
use crate::api::{self, Known, Table};
use crate::failure::Failure;
use crate::message::{Message, clipped};

/// The longest embed title Discord takes, in characters.
const TITLE_LIMIT: usize = 256;

/// The longest embed description Discord takes, in characters.
const DESCRIPTION_LIMIT: usize = 4096;

/// The longest footer the embed is given, in characters: with the title
/// and the description, within the 6,000 Discord takes for a whole embed.
const FOOTER_LIMIT: usize = 256;

/// Discord's translation table.
pub(super) const TABLE: Table = Table {
    party: "Discord",
    kind: error_code,
    wait,
    known,
};

/// The message that reports `message` on Discord.
pub(super) fn body(message: &Message) -> Value {
    let mut embed = json!({
        "footer": { "text": clipped(&context(message), FOOTER_LIMIT) },
    });
    if !message.title.trim().is_empty() {
        embed["title"] = json!(clipped(&message.title, TITLE_LIMIT));
    }
    if !message.text.trim().is_empty() {
        embed["description"] = json!(clipped(&message.text, DESCRIPTION_LIMIT));
    }
    if let Some(link) = message.link_without_credentials() {
        embed["url"] = json!(link.as_str());
    }
    if let Ok(at) = point_in_time::to_rfc3339(message.occurred_at) {
        embed["timestamp"] = json!(at);
    }
    json!({
        "embeds": [embed],
        "allowed_mentions": { "parse": [] },
    })
}

/// The JSON error `code` Discord's body gives, such as `10015`.
fn error_code(_: &HeaderMap, body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    Some(body.get("code")?.as_u64()?.to_string())
}

/// The wait a Discord answer asks for: the `retry_after` its body gives in
/// seconds with a fraction, else its `Retry-After`, rounded up to a whole
/// second.
fn wait(headers: &HeaderMap, body: &[u8], received: SystemTime) -> Option<Duration> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| body.get("retry_after")?.as_f64())
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .or_else(|| api::header_wait(headers, body, received))
}

/// The failure a Discord answer is, by its error code and else by its
/// status, if the table knows it: the codes of Discord's JSON error codes.
fn known(status: StatusCode, code: Option<&str>) -> Option<Known> {
    code.and_then(by_code)
        .map(Known::Is)
        .or_else(|| api::by_status(status))
}

/// The failure an error code Discord gives is, if the table knows it.
fn by_code(code: &str) -> Option<Failure> {
    match code {
        // Unknown Webhook.
        "10015" => Some(Failure::Gone),
        // Invalid Webhook Token; Missing Permissions.
        "50027" | "50013" => Some(Failure::Unauthorized),
        // Cannot send an empty message; Invalid Form Body.
        "50006" | "50035" => Some(Failure::Rejected),
        _ => None,
    }
}
