//! The `teams` channel's platform: a Microsoft Teams Workflows webhook's
//! message, one Adaptive Card (schema 1.4) with the title, the text, the
//! event's facts and an `Open` action on the link. A Workflows flow
//! answers a failure as `{"error": {"code": …}}`, which its table reads
//! first, then the status. An Office 365 connector's URL is refused when
//! the channel is made: Microsoft retired those connectors, and they take
//! no Adaptive Card.

use hyper::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use super::context;
use crate::api::{self, Known, Table};
use crate::failure::Failure;
use crate::message::Message;

/// Teams' translation table.
pub(super) const TABLE: Table = Table {
    party: "Microsoft Teams",
    kind: error_code,
    wait: api::header_wait,
    known,
};

/// Whether `host` is an Office 365 connector's: `outlook.office.com` or a
/// tenant's `*.webhook.office.com`.
pub(super) fn is_connector(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "outlook.office.com"
        || host == "webhook.office.com"
        || host.ends_with(".webhook.office.com")
}

/// The message that reports `message` on Teams.
pub(super) fn body(message: &Message) -> Value {
    let mut facts = vec![
        json!({ "title": "Event", "value": message.event.to_string() }),
        json!({ "title": "Product", "value": message.product.name() }),
    ];
    if let Some(run_id) = &message.run_id {
        facts.push(json!({ "title": "Run", "value": run_id.to_string() }));
    }
    if let Some(schedule_id) = &message.schedule_id {
        facts.push(json!({ "title": "Schedule", "value": schedule_id.as_str() }));
    }
    let mut card = json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.4",
        "fallbackText": context(message),
        "body": [
            {
                "type": "TextBlock",
                "text": message.title,
                "weight": "Bolder",
                "size": "Medium",
                "wrap": true,
            },
            { "type": "TextBlock", "text": message.text, "wrap": true },
            { "type": "FactSet", "facts": facts },
        ],
    });
    if let Some(link) = message.link_without_credentials() {
        card["actions"] =
            json!([{ "type": "Action.OpenUrl", "title": "Open", "url": link.as_str() }]);
    }
    json!({
        "type": "message",
        "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": card,
        }],
    })
}

/// The error `code` a Workflows flow's body gives, such as
/// `WorkflowTriggerIsNotEnabled`.
fn error_code(_: &HeaderMap, body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    Some(body.get("error")?.get("code")?.as_str()?.to_owned())
}

/// The failure a Workflows answer is, by its error code and else by its
/// status, if the table knows it.
fn known(status: StatusCode, code: Option<&str>) -> Option<Known> {
    code.and_then(by_code)
        .or(match status {
            StatusCode::BAD_REQUEST => Some(Failure::Rejected),
            StatusCode::NOT_FOUND | StatusCode::GONE => Some(Failure::Gone),
            _ => None,
        })
        .map(Known::Is)
        .or_else(|| api::by_status(status))
}

/// The failure an error code a Workflows flow gives is, if the table knows
/// it.
fn by_code(code: &str) -> Option<Failure> {
    match code {
        "TriggerInputSchemaMismatch" => Some(Failure::Rejected),
        "WorkflowTriggerIsNotEnabled" | "WorkflowNotFound" => Some(Failure::Gone),
        "DirectApiAuthorizationRequired" => Some(Failure::Unauthorized),
        _ => None,
    }
}
