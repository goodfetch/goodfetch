//! Each type with fields of its own beside its members, as schemars writes
//! a struct that flattens a tagged union, as `allOf` the two.
//!
//! `json-schema-to-typescript` reads such a type, when no other type refers
//! to it, as its members alone, and writes its fields as a separate
//! interface no type names, as it does for an Error code's details; as the
//! intersection of the two it writes both. One another type refers to it
//! reads right, as the intersection of its fields and its members, named
//! apart, and stays as it is: as `allOf`, the names it gives the types it
//! holds may change.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// The keywords that describe a type's own fields, which go to the first
/// part of its intersection.
const FIELD_KEYWORDS: [&str; 4] = ["type", "properties", "required", "additionalProperties"];

/// The keywords that list a type's members, which go to the second part.
const MEMBER_KEYWORDS: [&str; 2] = ["oneOf", "anyOf"];

/// `input`, a generator input, with each definition that has both fields
/// and members, and that no other definition refers to, as `allOf` the two,
/// its other keywords, such as its description, kept on it.
pub(super) fn fields_beside_members_as_intersections(input: &mut Value) {
    let mut referred_to = BTreeSet::new();
    if let Some(definitions) = input.get("$defs") {
        referred_names(definitions, &mut referred_to);
    }
    let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) else {
        return;
    };
    for (name, schema) in definitions.iter_mut() {
        let Some(object) = schema.as_object_mut() else {
            continue;
        };
        if referred_to.contains(name)
            || !object.contains_key("properties")
            || !MEMBER_KEYWORDS.iter().any(|key| object.contains_key(*key))
        {
            continue;
        }
        let fields = take(object, &FIELD_KEYWORDS);
        let members = take(object, &MEMBER_KEYWORDS);
        object.insert(
            "allOf".to_owned(),
            Value::Array(vec![Value::Object(fields), Value::Object(members)]),
        );
    }
}

/// Adds to `names` each definition a `$ref` in `value` names.
fn referred_names(value: &Value, names: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(name) = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
            {
                names.insert(name.to_owned());
            }
            for child in object.values() {
                referred_names(child, names);
            }
        }
        Value::Array(items) => {
            for item in items {
                referred_names(item, names);
            }
        }
        _ => {}
    }
}

fn take(object: &mut Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|key| object.remove(*key).map(|value| ((*key).to_owned(), value)))
        .collect()
}

#[cfg(test)]
mod tests;
