//! The protocol's duration in a generator input mapped to the client
//! package's, as a function of the input.

use serde_json::json;

use super::{DURATION, use_client_type};

/// The protocol's duration leaves `$defs`, and every reference to it, at any
/// depth and beside a description of its own, names the client package's
/// type instead; any other schema, a bare integer included, stays.
#[test]
fn every_duration_becomes_the_client_packages_duration() {
    let mut input = json!({
        "$defs": {
            "Milliseconds": {
                "description": "A duration, in milliseconds.",
                "type": "integer",
                "format": "uint64",
                "minimum": 0,
            },
            "RunUsage": {
                "type": "object",
                "properties": {
                    "browser_time": { "$ref": "#/$defs/Milliseconds", "description": "Held." },
                    "waits": {
                        "type": "array",
                        "items": { "anyOf": [{ "$ref": "#/$defs/Milliseconds" }, { "type": "null" }] },
                    },
                    "pages": { "type": "integer", "format": "uint64", "minimum": 0 },
                },
            },
        },
    });

    use_client_type(&mut input);

    assert_eq!(
        input,
        json!({
            "$defs": {
                "RunUsage": {
                    "type": "object",
                    "properties": {
                        "browser_time": { "customTypePath": DURATION, "description": "Held." },
                        "waits": {
                            "type": "array",
                            "items": { "anyOf": [{ "customTypePath": DURATION }, { "type": "null" }] },
                        },
                        "pages": { "type": "integer", "format": "uint64", "minimum": 0 },
                    },
                },
            },
        })
    );
}

/// Foxhound's layer, which refers to Collie's types by name, imports the
/// duration from where Collie's layer does, and nothing else from here.
#[test]
fn only_the_protocols_duration_names_the_client_packages_type() {
    assert_eq!(super::client_type("Milliseconds"), Some(DURATION));
    assert_eq!(super::client_type("RunUsage"), None);
}
