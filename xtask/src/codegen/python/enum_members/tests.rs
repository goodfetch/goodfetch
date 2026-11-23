//! Each closed value union's member names, as a function of the generator
//! input.

use serde_json::json;

use super::with_free_member_names;

/// A union of constants holding a name `StrEnum` takes becomes an `enum`
/// whose member of that name ends with an underscore, each value and
/// description kept; a union without one, and any other type, stays.
#[test]
fn a_value_union_holding_a_name_strenum_takes_renames_that_member() {
    let mut input = json!({
        "$defs": {
            "Refused": {
                "description": "What was refused.",
                "oneOf": [
                    { "const": "key", "type": "string", "description": "The key." },
                    { "const": "value", "type": "string", "description": "The value." },
                ],
            },
            "Claim": {
                "oneOf": [
                    { "const": "done", "type": "string", "description": "Done." },
                ],
            },
            "Tagged": {
                "oneOf": [{ "type": "object", "properties": { "kind": { "const": "value" } } }],
            },
        },
    });
    let unchanged = input.clone();

    with_free_member_names(&mut input);

    assert_eq!(
        input["$defs"]["Refused"],
        json!({
            "description": "What was refused.",
            "type": "string",
            "enum": ["key", "value"],
            "x-enum-varnames": ["key", "value_"],
            "x-enum-descriptions": ["The key.", "The value."],
        })
    );
    assert_eq!(input["$defs"]["Claim"], unchanged["$defs"]["Claim"]);
    assert_eq!(input["$defs"]["Tagged"], unchanged["$defs"]["Tagged"]);
}
