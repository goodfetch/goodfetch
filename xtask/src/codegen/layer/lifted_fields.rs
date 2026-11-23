//! Each Error code's details type without the fields every error carries.
//!
//! The TypeScript and Python SDKs take those fields out of a reply's `data`
//! onto the error itself, each when its value has the field's shape
//! (`fieldsOfData` in the TypeScript client package's `errors.ts`,
//! `_fields_of_data` in the Python one's `_rpc.py`): the rest is the code's
//! `details`. A details type the SDKs read typed holds that rest alone, so
//! it requires no field its error already took (ADR 0079).

use anyhow::{Result, bail};
use serde_json::{Map, Value};

/// How the SDKs know a field every error carries: the shape of its value.
#[derive(Clone, Copy)]
enum Shape {
    /// Text.
    Text,
    /// A setting's limit: an object with a text `key` and `layer`.
    SettingLimit,
    /// A list of lines of text.
    Lines,
    /// Any object.
    Object,
}

/// The fields the SDKs take out of an error's `data`, by wire name, each
/// with the shape its value must have to be taken.
const LIFTED: &[(&str, Shape)] = &[
    ("fix", Shape::Text),
    ("operation", Shape::Text),
    ("call_id", Shape::Text),
    ("run_id", Shape::Text),
    ("target", Shape::Text),
    ("url", Shape::Text),
    ("observed", Shape::Text),
    ("limit", Shape::SettingLimit),
    ("call_log", Shape::Lines),
    ("usage", Shape::Object),
    ("agent_task", Shape::Object),
];

/// `definitions` with each of `details`, the codes' details types, without
/// the fields the SDKs take out of an error's data, in the type and in each
/// of its members, out of its fields and those it requires. A field taken
/// out for some of its values only is refused, as is a details type that
/// loses a field while another definition holds it, which keeps its fields.
pub(super) fn without_lifted_fields<'a>(
    definitions: &mut Map<String, Value>,
    details: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    for name in details {
        let Some(schema) = definitions.get(name) else {
            continue;
        };
        let parts = std::iter::once(schema).chain(members(schema));
        let mut lifted = Vec::new();
        for (index, part) in parts.enumerate() {
            for (field, shape) in LIFTED {
                let Some(field_schema) = part.pointer(&format!("/properties/{field}")) else {
                    continue;
                };
                if has_shape(*shape, field_schema, definitions)
                    .map_err(|reason| anyhow::anyhow!("`{name}`'s field `{field}` {reason}"))?
                {
                    lifted.push((index, *field));
                }
            }
        }
        if lifted.is_empty() {
            continue;
        }
        let reference = format!("#/$defs/{name}");
        if let Some(holder) = definitions
            .iter()
            .find(|(_, other)| refers_to(other, &reference))
            .map(|(holder, _)| holder)
        {
            bail!(
                "`{name}` is a code's details type that `{holder}` also holds, and the SDKs take its fields {} out of the error's data: give the details a type of their own",
                lifted
                    .iter()
                    .map(|(_, field)| format!("`{field}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let schema = definitions
            .get_mut(name)
            .expect("invariant: the details type was found above");
        let union = members_key(schema);
        for (index, field) in lifted {
            let part = if index == 0 {
                &mut *schema
            } else {
                schema
                    .get_mut(union)
                    .and_then(|members| members.get_mut(index - 1))
                    .expect("invariant: the member was found above")
            };
            remove_field(part, field);
        }
    }
    Ok(())
}

/// The keyword that lists a schema's members: `oneOf`, unless it has none
/// and `anyOf` does.
fn members_key(schema: &Value) -> &'static str {
    if schema.get("oneOf").is_none() && schema.get("anyOf").is_some() {
        "anyOf"
    } else {
        "oneOf"
    }
}

/// The members of a schema, which may hold fields of its own beside them.
fn members(schema: &Value) -> impl Iterator<Item = &Value> {
    schema
        .get(members_key(schema))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn remove_field(schema: &mut Value, field: &str) {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.remove(field);
    }
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|name| name.as_str() != Some(field));
        if required.is_empty() {
            object.remove("required");
        }
    }
}

/// Whether every value of `schema` but null has `shape`; an error, its
/// reason, when some do and others do not.
fn has_shape(
    shape: Shape,
    schema: &Value,
    definitions: &Map<String, Value>,
) -> Result<bool, String> {
    let schema = resolved(schema, definitions);
    let alternatives = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
        .filter(|_| schema.get("properties").is_none());
    if let Some(alternatives) = alternatives {
        let mut answers = alternatives
            .iter()
            .filter(|alternative| !is_null(resolved(alternative, definitions)))
            .map(|alternative| has_shape(shape, alternative, definitions));
        let Some(first) = answers.next().transpose()? else {
            return Ok(false);
        };
        for answer in answers {
            if answer? != first {
                return Err(
                    "is taken out of the error's data for some of its values only".to_owned(),
                );
            }
        }
        return Ok(first);
    }
    let kind = match schema.get("type") {
        Some(Value::String(kind)) => Some(kind.as_str()),
        Some(Value::Array(kinds)) => {
            let kinds: Vec<&str> = kinds
                .iter()
                .filter_map(Value::as_str)
                .filter(|kind| *kind != "null")
                .collect();
            match kinds.as_slice() {
                [kind] => Some(*kind),
                [] => None,
                _ if kinds.contains(&expected(shape)) => {
                    return Err(
                        "is taken out of the error's data for some of its values only".to_owned(),
                    );
                }
                _ => None,
            }
        }
        _ => None,
    };
    if kind != Some(expected(shape)) {
        return Ok(false);
    }
    match shape {
        Shape::Text | Shape::Object => Ok(true),
        Shape::Lines => schema.get("items").map_or(Ok(false), |items| {
            has_shape(Shape::Text, items, definitions)
        }),
        Shape::SettingLimit => {
            for key in ["key", "layer"] {
                let required = schema
                    .get("required")
                    .and_then(Value::as_array)
                    .is_some_and(|required| required.iter().any(|name| name == key));
                let text = schema
                    .pointer(&format!("/properties/{key}"))
                    .map_or(Ok(false), |field| {
                        has_shape(Shape::Text, field, definitions)
                    })?;
                if !(required && text) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

/// The JSON type a value of `shape` has.
fn expected(shape: Shape) -> &'static str {
    match shape {
        Shape::Text => "string",
        Shape::Lines => "array",
        Shape::SettingLimit | Shape::Object => "object",
    }
}

/// `schema`, or the definition it refers to, followed to the end.
fn resolved<'a>(schema: &'a Value, definitions: &'a Map<String, Value>) -> &'a Value {
    let mut schema = schema;
    while let Some(name) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
    {
        match definitions.get(name) {
            Some(definition) => schema = definition,
            None => break,
        }
    }
    schema
}

fn is_null(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
}

fn refers_to(value: &Value, reference: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.get("$ref").and_then(Value::as_str) == Some(reference)
                || object.values().any(|child| refers_to(child, reference))
        }
        Value::Array(items) => items.iter().any(|item| refers_to(item, reference)),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
