//! A value union both Products share as one layer spells it, as a function
//! of the definitions and the layer's values.

use goodfetch_protocol::catalog::Product;
use serde_json::{Map, Value, json};

use super::{SharedTypeValue, is_value_union, with_own_values};

fn definitions(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(definitions) => definitions,
        _ => unreachable!("the definitions are an object"),
    }
}

fn value(shared_type: &str, name: &str) -> SharedTypeValue {
    SharedTypeValue {
        name: name.to_owned(),
        product: Product::Collie,
        shared_type: shared_type.to_owned(),
        description: String::new(),
    }
}

/// A shared value union keeps the layer's values alone; a type the layer's
/// values name that is no value union, such as an object whose kind they
/// list, stays, as does every other type.
#[test]
fn a_shared_value_union_keeps_the_layers_values_alone() {
    let cause = |name: &str| json!({ "const": name, "type": "string", "description": name });
    let mut input = definitions(json!({
        "BlockCause": { "oneOf": [cause("captcha"), cause("decoy"), cause("ip_block")] },
        "LimitScope": {
            "oneOf": [{ "type": "object", "properties": { "kind": { "const": "crawl" } } }],
        },
        "Claim": { "oneOf": [cause("done"), cause("gave_up")] },
    }));
    let unchanged = input.clone();

    with_own_values(
        &mut input,
        &[
            value("block_cause", "captcha"),
            value("block_cause", "ip_block"),
            value("limit_scope", "run"),
        ],
    );

    assert_eq!(
        input["BlockCause"],
        json!({ "oneOf": [cause("captcha"), cause("ip_block")] })
    );
    assert_eq!(input["LimitScope"], unchanged["LimitScope"]);
    assert_eq!(input["Claim"], unchanged["Claim"]);
    assert!(is_value_union(&input["Claim"]));
    assert!(!is_value_union(&input["LimitScope"]));
}
