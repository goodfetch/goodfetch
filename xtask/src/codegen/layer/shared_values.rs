//! The value unions both Products share (ADR 0095), such as `BlockCause`,
//! as each layer spells them.
//!
//! The bundle holds one such union with every Product's values. Collie's
//! layer names Collie's alone, as its `shared-types` module does, any other
//! value an engine sends kept as received (ADR 0079); Foxhound's layer keeps
//! its own definition, with every value, rather than take Collie's.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::SharedTypeValue;
use crate::codegen::pascal;

/// Whether `schema` is a union of text values, each a `const`.
pub(super) fn is_value_union(schema: &Value) -> bool {
    schema
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|members| members.iter().all(is_value))
}

/// `definitions` with each value union a shared type of `values` names
/// holding the values `values` lists for it, and no other.
pub(super) fn with_own_values(definitions: &mut Map<String, Value>, values: &[SharedTypeValue]) {
    let mut own: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for value in values {
        own.entry(pascal(&value.shared_type))
            .or_default()
            .insert(value.name.as_str());
    }
    for (name, kept) in own {
        let Some(schema) = definitions.get_mut(&name) else {
            continue;
        };
        if !is_value_union(schema) {
            continue;
        }
        if let Some(members) = schema.get_mut("oneOf").and_then(Value::as_array_mut) {
            members.retain(|member| {
                member
                    .get("const")
                    .and_then(Value::as_str)
                    .is_some_and(|value| kept.contains(value))
            });
        }
    }
}

fn is_value(member: &Value) -> bool {
    member.get("const").is_some_and(Value::is_string)
}

#[cfg(test)]
mod tests;
