use serde_json::json;

use super::{OpenUnion, OpenValues, open_definitions};

fn verdict(open: bool) -> serde_json::Value {
    let mut schema = json!({
        "description": "Whether a Run's checks prove it.",
        "oneOf": [
            { "const": "verified", "description": "Every check\npassed.", "type": "string" },
            { "const": "rejected", "description": "A check failed.", "type": "string" }
        ]
    });
    if open {
        schema["open"] = json!(true);
    }
    schema
}

#[test]
fn an_enumeration_marked_open_gives_its_values_in_order() {
    let open = OpenValues::of(&verdict(true)).unwrap();

    assert_eq!(open.literals(), ["\"verified\"", "\"rejected\""]);
    assert_eq!(
        open.documentation(),
        "Whether a Run's checks prove it.\n\nA value this package does not know, which a newer engine may send, is kept as received (ADR 0079).\n\n- `verified`: Every check passed.\n- `rejected`: A check failed."
    );
}

#[test]
fn an_enumeration_not_marked_open_stays_closed() {
    assert!(OpenValues::of(&verdict(false)).is_none());
}

#[test]
fn typescript_types_an_open_enumeration_as_its_values_or_any_string() {
    let mut input = json!({ "$defs": { "Verdict": verdict(true), "Closed": verdict(false) } });

    super::typescript_input(&mut input);

    assert_eq!(
        input["$defs"]["Verdict"]["tsType"],
        "\"verified\" | \"rejected\" | (string & {})"
    );
    assert_eq!(input["$defs"]["Closed"], verdict(false));
}

#[test]
fn python_names_its_alias_wherever_an_open_enumeration_is_referred_to() {
    let mut input = json!({ "$defs": {
        "Verdict": verdict(true),
        "Report": { "properties": {
            "verdict": { "$ref": "#/$defs/Verdict" },
            "earlier": { "type": "array", "items": { "$ref": "#/$defs/Verdict" } }
        } }
    } });

    let open = super::python_input(&mut input, "package.open_values");

    assert_eq!(open.len(), 1);
    assert!(input["$defs"].get("Verdict").is_none());
    let properties = &input["$defs"]["Report"]["properties"];
    assert_eq!(
        properties["verdict"],
        json!({ "customTypePath": "package.open_values.Verdict" })
    );
    assert_eq!(
        properties["earlier"]["items"],
        json!({ "customTypePath": "package.open_values.Verdict" })
    );
}

#[test]
fn only_the_open_definitions_are_listed() {
    let definitions = json!({ "Verdict": verdict(true), "Closed": verdict(false) });

    let names: Vec<String> = open_definitions(definitions.as_object().unwrap())
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    assert_eq!(names, ["Verdict"]);
}

/// A tagged union of two members, `abort` with a field and `pause`.
fn stop_reason(open: bool) -> serde_json::Value {
    let mut schema = json!({
        "description": "Why a Run stopped.",
        "oneOf": [
            {
                "properties": {
                    "by": { "type": "string" },
                    "kind": { "const": "abort", "type": "string" }
                },
                "required": ["kind", "by"],
                "type": "object"
            },
            {
                "properties": { "kind": { "const": "pause", "type": "string" } },
                "required": ["kind"],
                "type": "object"
            }
        ]
    });
    if open {
        schema["open"] = json!(true);
    }
    schema
}

#[test]
fn a_tagged_union_marked_open_gives_its_tag_and_its_members_in_order() {
    let union = OpenUnion::of(&stop_reason(true)).unwrap();

    assert_eq!(
        (union.tag.as_str(), union.known),
        ("kind", vec!["abort".to_owned(), "pause".to_owned()])
    );
}

#[test]
fn a_tagged_union_not_marked_open_and_an_open_enumeration_are_no_open_union() {
    assert!(OpenUnion::of(&stop_reason(false)).is_none());
    assert!(OpenUnion::of(&verdict(true)).is_none());
}

#[test]
fn an_open_union_gets_an_unknown_member_keeping_the_tag_and_the_rest() {
    let mut input =
        json!({ "$defs": { "StopReason": stop_reason(true), "Closed": stop_reason(false) } });

    super::unknown_members(&mut input, None).unwrap();

    let members = input["$defs"]["StopReason"]["oneOf"].as_array().unwrap();
    assert_eq!(members.len(), 3);
    let unknown = &members[2];
    assert_eq!(
        (
            &unknown["properties"]["kind"],
            &unknown["properties"]["tag"]["type"],
            &unknown["required"],
            &unknown["additionalProperties"],
        ),
        (
            &json!({ "const": "unknown", "type": "string" }),
            &json!("string"),
            &json!(["kind", "tag"]),
            &json!(true),
        )
    );
    assert!(unknown.get("customBasePath").is_none());
    assert_eq!(input["$defs"]["Closed"], stop_reason(false));
}

#[test]
fn python_builds_the_unknown_member_on_the_runtimes_base() {
    let mut input = json!({ "$defs": { "StopReason": stop_reason(true) } });

    super::unknown_members(&mut input, Some("package.unknown_member.UnknownMember")).unwrap();

    assert_eq!(
        input["$defs"]["StopReason"]["oneOf"][2]["customBasePath"],
        "package.unknown_member.UnknownMember"
    );
}

#[test]
fn an_open_union_that_already_names_a_member_unknown_or_a_field_tag_is_refused() {
    let mut named_unknown = stop_reason(true);
    named_unknown["oneOf"][1]["properties"]["kind"]["const"] = json!("unknown");
    let mut field_tag = stop_reason(true);
    field_tag["oneOf"][0]["properties"]["tag"] = json!({ "type": "string" });

    for schema in [named_unknown, field_tag] {
        let mut input = json!({ "$defs": { "StopReason": schema } });
        assert!(super::unknown_members(&mut input, None).is_err());
    }
}

#[test]
fn a_schema_marked_open_that_is_neither_an_enumeration_nor_a_tagged_union_is_refused() {
    let mut input = json!({ "$defs": { "Odd": { "open": true, "type": "object" } } });

    assert!(super::unknown_members(&mut input, None).is_err());
}
