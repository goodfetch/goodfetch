use serde_json::json;

use super::{OpenValues, open_definitions};

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
