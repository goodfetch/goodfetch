//! The Notification channel's cases (`conformance/notification/cases.toml`):
//! the event each case sends with `notification.send`, and the checks of what
//! an implementation answers for it.

use goodfetch_protocol::plugin::notification::{NOTIFICATION_SEND, NotificationSendResult};
use goodfetch_protocol::result_document::CaseCheck;
use serde::Deserialize as _;
use serde_json::{Value, json};

/// The method every case calls.
pub(super) const METHOD: &str = NOTIFICATION_SEND;

/// The params of `notification.send`, with a field a newer engine adds when
/// `newer`.
pub(super) fn params(newer: bool) -> Value {
    let event_id = if newer {
        "conformance-evt-newer"
    } else {
        "conformance-evt-1"
    };
    let mut params = json!({
        "event_id": event_id,
        "event": "run_failed",
        "text": "The conformance Run stopped on an error.",
        "link": "http://127.0.0.1:7777/runs/conformance",
    });
    if newer {
        params["added_in_a_minor"] = json!(1);
    }
    params
}

/// The checks of `answer`, what an implementation answered for
/// `notification.send`.
pub(super) fn checks(answer: &Value) -> Vec<CaseCheck> {
    let result = answer
        .get("result")
        .and_then(|result| NotificationSendResult::deserialize(result).ok());
    vec![
        CaseCheck {
            name: "the answer is `{ delivered }` and no other field".to_owned(),
            passed: result.is_some(),
        },
        CaseCheck {
            name: "the Notification is reported delivered".to_owned(),
            passed: result.is_some_and(|result| result.delivered),
        },
    ]
}
