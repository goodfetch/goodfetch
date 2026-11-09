//! What the test Client's call loops answer on a handler's behalf. Collie's
//! tests take this crate too, so it reads Foxhound's handler requests from
//! `foxhound_names`, never from the `foxhound` module (ADR 0099).

use goodfetch_protocol::foxhound_names;
use serde_json::Value;

/// Whether the engine's `message` asks the Client's handler for an answer.
pub(crate) fn is_handler_request(message: &Value) -> bool {
    message["method"]
        .as_str()
        .is_some_and(|method| foxhound_names::HANDLER_REQUESTS.contains(&method))
}
