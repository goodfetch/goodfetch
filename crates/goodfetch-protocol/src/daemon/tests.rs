//! A Daemon's token in its contracts: plain text in JSON and in its
//! schema, never in `Debug` (STYLE.md).

use schemars::SchemaGenerator;
use serde_json::json;

use super::{DaemonToken, HttpListenerToken};

#[test]
fn a_token_prints_in_json_and_never_in_debug() {
    let printed = HttpListenerToken {
        token: DaemonToken::new("not-a-real-token"),
    };

    let (debug, json) = (format!("{printed:?}"), serde_json::to_value(&printed));

    assert_eq!(
        (debug.contains("not-a-real-token"), json.ok()),
        (false, Some(json!({ "token": "not-a-real-token" })))
    );
}

#[test]
fn a_tokens_schema_is_plain_text() {
    let mut generator = SchemaGenerator::default();

    let schema = generator.subschema_for::<DaemonToken>();

    assert_eq!(
        serde_json::to_value(schema).ok(),
        Some(json!({ "type": "string" }))
    );
}
