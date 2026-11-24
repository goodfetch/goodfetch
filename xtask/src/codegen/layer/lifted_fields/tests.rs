//! Each details type without the fields the SDKs take out of an error's
//! data, as a function of the definitions.

use serde_json::{Value, json};

use super::without_lifted_fields;

/// A field the SDKs take out, by its name and shape, leaves the type's
/// fields and those it requires, in the type and in each of its members,
/// through references and when it may be null; a field of that name and
/// another shape stays, as do the type's other fields and other types.
#[test]
fn a_details_type_loses_each_field_the_sdks_take_out_of_the_errors_data() {
    let Value::Object(mut input) = json!({
        "SettingLimit": {
            "type": "object",
            "properties": {
                "key": { "type": "string" },
                "layer": { "$ref": "#/$defs/SettingsLayer" },
            },
            "required": ["key", "layer"],
        },
        "SettingsLayer": {
            "oneOf": [
                { "const": "default", "type": "string" },
                { "const": "run", "type": "string" },
            ],
        },
        "RunId": { "type": "string", "pattern": "^[a-z]+$" },
        "FetchDetails": {
            "type": "object",
            "properties": {
                "fix": { "type": "string" },
                "url": { "type": ["string", "null"] },
                "run_id": { "anyOf": [{ "$ref": "#/$defs/RunId" }, { "type": "null" }] },
                "limit": { "$ref": "#/$defs/SettingLimit" },
                "call_log": { "type": "array", "items": { "type": "string" } },
                "usage": { "type": "object", "properties": {} },
                "host": { "type": "string" },
            },
            "required": ["fix", "limit", "host"],
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "failure": { "const": "status", "type": "string" },
                        "target": { "type": "string" },
                    },
                    "required": ["failure", "target"],
                },
            ],
        },
        "BytesDetails": {
            "type": "object",
            "properties": {
                "limit": { "type": "integer" },
                "observed": { "type": "integer" },
            },
            "required": ["limit", "observed"],
        },
        "Other": {
            "type": "object",
            "properties": { "url": { "type": "string" } },
            "required": ["url"],
        },
    }) else {
        unreachable!("the definitions are an object")
    };

    without_lifted_fields(&mut input, ["FetchDetails", "BytesDetails"]).unwrap();

    assert_eq!(
        input["FetchDetails"],
        json!({
            "type": "object",
            "properties": { "host": { "type": "string" } },
            "required": ["host"],
            "oneOf": [
                {
                    "type": "object",
                    "properties": { "failure": { "const": "status", "type": "string" } },
                    "required": ["failure"],
                },
            ],
        })
    );
    assert_eq!(
        input["BytesDetails"]["required"],
        json!(["limit", "observed"])
    );
    assert_eq!(input["Other"]["required"], json!(["url"]));
}

/// A details type another type also holds keeps its fields there, so one
/// with a field the SDKs take out is refused rather than changed for both.
#[test]
fn a_details_type_another_type_holds_is_refused_when_it_loses_a_field() {
    let Value::Object(mut input) = json!({
        "StopDetails": {
            "type": "object",
            "properties": { "fix": { "type": "string" } },
        },
        "Stop": {
            "type": "object",
            "properties": { "details": { "$ref": "#/$defs/StopDetails" } },
        },
    }) else {
        unreachable!("the definitions are an object")
    };

    let refused = without_lifted_fields(&mut input, ["StopDetails"])
        .unwrap_err()
        .to_string();

    assert!(refused.contains("`StopDetails`"), "{refused}");
}

/// A field the SDKs take out for some of its values only, such as a `url`
/// that is text or a number, is refused: its type would be wrong for one.
#[test]
fn a_field_taken_out_for_some_values_only_is_refused() {
    let Value::Object(mut input) = json!({
        "OddDetails": {
            "type": "object",
            "properties": {
                "url": { "anyOf": [{ "type": "string" }, { "type": "integer" }] },
            },
        },
    }) else {
        unreachable!("the definitions are an object")
    };

    let refused = without_lifted_fields(&mut input, ["OddDetails"])
        .unwrap_err()
        .to_string();

    assert!(refused.contains("`url`"), "{refused}");
}
