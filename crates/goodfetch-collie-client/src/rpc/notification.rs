//! The notifications the Connection sends of its own about a call, a
//! `$/cancel` or a `$/credit`, and each notification as the one line it is
//! written as.

use goodfetch_protocol::connection::{CANCEL, CancelParams};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::jsonrpc::{Notification, RequestId, Version};

use crate::line;
use crate::logging;

/// The `$/cancel` of the call `id`.
pub(super) fn cancelled(id: i64) -> Notification {
    Notification {
        jsonrpc: Version,
        method: CANCEL.to_owned(),
        params: serde_json::to_value(CancelParams {
            id: RequestId::Number(id),
        })
        .ok(),
    }
}

/// The `$/credit` that grants the call `id` `credits` more items.
pub(super) fn credited(id: i64, credits: u32) -> Notification {
    Notification {
        jsonrpc: Version,
        method: CREDIT.to_owned(),
        params: serde_json::to_value(CreditParams {
            id: RequestId::Number(id),
            credits,
        })
        .ok(),
    }
}

/// `notification` as one line, or none, with a warning, when it is no
/// JSON.
pub(super) fn encoded(notification: &Notification) -> Option<Vec<u8>> {
    let encoded = line::encode(notification);
    if encoded.is_err() {
        tracing::warn!(target: logging::CLIENT, method = %notification.method, "a notification that is no JSON was not sent");
    }
    encoded.ok()
}
