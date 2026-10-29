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

/// The token's schema is the plain text the field held before the
/// newtype, inline, so the bundle names no type for it.
#[test]
fn a_tokens_schema_is_plain_text() {
    let mut generator = SchemaGenerator::default();

    let schema = serde_json::to_value(generator.subschema_for::<HttpListenerToken>()).ok();
    let token = generator
        .definitions()
        .get("HttpListenerToken")
        .map(|definition| definition["properties"]["token"].clone());

    assert_eq!(
        (
            token.map(|token| (token["type"].clone(), token.get("$ref").is_some())),
            generator.definitions().contains_key("DaemonToken"),
            schema,
        ),
        (
            Some((json!("string"), false)),
            false,
            Some(json!({ "$ref": "#/$defs/HttpListenerToken" })),
        )
    );
}
