//! Each type with fields beside its members as the intersection of the two,
//! as a function of the generator input.

use serde_json::json;

use super::fields_beside_members_as_intersections;

/// A type with fields of its own beside its members, as the protocol's
/// flattened tagged unions are, that no other type refers to becomes
/// `allOf` the fields and the members, its description and other keywords
/// kept; one another type refers to stays, as does a type without both.
#[test]
fn a_type_with_fields_beside_its_members_becomes_their_intersection() {
    let mut input = json!({
        "$defs": {
            "DownloadDetails": {
                "description": "What a download's failure details.",
                "type": "object",
                "properties": { "host": { "type": "string" } },
                "required": ["host"],
                "open": true,
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "failure": { "const": "network", "type": "string" } },
                        "required": ["failure"],
                    },
                ],
            },
            "Plain": {
                "type": "object",
                "properties": { "host": { "type": "string" } },
            },
            "Outcome": {
                "type": "object",
                "properties": { "attempts": { "type": "integer" } },
                "oneOf": [{ "type": "object", "properties": { "kind": { "const": "done" } } }],
            },
            "Ended": {
                "type": "object",
                "properties": { "outcome": { "$ref": "#/$defs/Outcome" } },
            },
        },
    });
    let referred_to = ["Outcome", "Ended"].map(|name| (name, input["$defs"][name].clone()));

    fields_beside_members_as_intersections(&mut input);

    for (name, schema) in referred_to {
        assert_eq!(input["$defs"][name], schema, "{name}");
        input["$defs"]
            .as_object_mut()
            .expect("the definitions are an object")
            .remove(name);
    }
    assert_eq!(
        input,
        json!({
            "$defs": {
                "DownloadDetails": {
                    "description": "What a download's failure details.",
                    "open": true,
                    "allOf": [
                        {
                            "type": "object",
                            "properties": { "host": { "type": "string" } },
                            "required": ["host"],
                        },
                        {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "failure": { "const": "network", "type": "string" },
                                    },
                                    "required": ["failure"],
                                },
                            ],
                        },
                    ],
                },
                "Plain": {
                    "type": "object",
                    "properties": { "host": { "type": "string" } },
                },
            },
        })
    );
}
