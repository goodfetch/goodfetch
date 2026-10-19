//! Fills a JSON Schema from a task's Records: an object takes each property
//! from the first Record's field of the same name, an array takes one item
//! per Record, and a value the Records lack is its type's empty value.

use serde_json::{Map, Number, Value};

type Fields = Map<String, Value>;

/// The value `schema` describes, filled from `records`.
pub(crate) fn fill(schema: &Value, records: &[Fields]) -> Value {
    value(schema, records, records.first(), None)
}

/// The schema's type, in lowercase: its `type` (the first that is not
/// `null` when it lists several), else what its keywords imply.
fn kind(schema: &Value) -> String {
    let named = match &schema["type"] {
        Value::String(kind) => Some(kind.to_ascii_lowercase()),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_ascii_lowercase)
            .find(|kind| kind != "null"),
        _ => None,
    };
    named.unwrap_or_else(|| {
        if schema.get("properties").is_some() {
            "object".to_owned()
        } else if schema.get("items").is_some() {
            "array".to_owned()
        } else if let Some(first) = schema["anyOf"].get(0).or_else(|| schema["oneOf"].get(0)) {
            kind(first)
        } else {
            "string".to_owned()
        }
    })
}

/// The value of `schema` for `record`, the field `name` when it is a
/// property.
fn value(schema: &Value, records: &[Fields], record: Option<&Fields>, name: Option<&str>) -> Value {
    match kind(schema).as_str() {
        "object" => Value::Object(
            schema["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(property, sub)| {
                    (
                        property.clone(),
                        value(sub, records, record, Some(property)),
                    )
                })
                .collect(),
        ),
        "array" => Value::Array(
            records
                .iter()
                .map(|each| value(&schema["items"], records, Some(each), None))
                .collect(),
        ),
        kind => scalar(kind, name.and_then(|name| record?.get(name))),
    }
}

/// `field` as a value of `kind`, or the kind's empty value.
fn scalar(kind: &str, field: Option<&Value>) -> Value {
    let text = field.map(|field| match field {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    });
    match kind {
        "number" | "integer" => text
            .and_then(|text| text.parse::<Number>().ok())
            .map_or(Value::Number(Number::from(0)), Value::Number),
        "boolean" => Value::Bool(text.is_some_and(|text| text == "true")),
        "null" => Value::Null,
        _ => Value::String(text.unwrap_or_default()),
    }
}
