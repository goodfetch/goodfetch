//! The open enumerations of the protocol (ADR 0079): a `snake_case` string
//! type whose schema says `"open": true`, such as a Verdict or a Block
//! cause, to which a newer engine may add a value. The SDKs keep such a
//! value as received, so each layer types it as its known values or any
//! other string: `Known | (string & {})` in TypeScript and
//! `Literal[...] | str` in Python, never a closed union or enum.

use std::fmt::Write as _;

use serde_json::{Map, Value};

/// The schema keyword that marks an enumeration open.
const OPEN: &str = "open";

/// An open enumeration's description and values, each with its own.
pub(super) struct OpenValues {
    pub(super) description: String,
    pub(super) values: Vec<(String, String)>,
}

impl OpenValues {
    /// `schema`'s, when it is an open enumeration: a `oneOf` of string
    /// constants marked `"open": true`.
    pub(super) fn of(schema: &Value) -> Option<Self> {
        let schema = schema.as_object()?;
        if schema.get(OPEN).and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let values = schema
            .get("oneOf")?
            .as_array()?
            .iter()
            .map(|value| {
                let value = value.as_object()?;
                let name = value.get("const")?.as_str()?;
                let description = text(value, "description");
                Some((name.to_owned(), description))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            description: text(schema, "description"),
            values,
        })
    }

    /// Its description, which says how an unknown value reads, followed by
    /// each value's own, as a layer's doc comment gives it.
    pub(super) fn documentation(&self) -> String {
        let mut text = format!(
            "{}\n\nA value this package does not know, which a newer engine may send, is kept as received (ADR 0079).\n",
            self.description.trim_end()
        );
        for (value, description) in &self.values {
            let _ = write!(text, "\n- `{value}`: {}", one_line(description));
        }
        text
    }

    /// Its known values, each as a quoted string literal, in order.
    pub(super) fn literals(&self) -> Vec<String> {
        self.values
            .iter()
            .map(|(value, _)| format!("{value:?}"))
            .collect()
    }
}

/// The open enumerations among `definitions`, by name.
pub(super) fn open_definitions(definitions: &Map<String, Value>) -> Vec<(String, OpenValues)> {
    definitions
        .iter()
        .filter_map(|(name, schema)| OpenValues::of(schema).map(|open| (name.clone(), open)))
        .collect()
}

/// `input`'s open enumerations typed for `json-schema-to-typescript`: each
/// a `tsType` of its known values or any other string, `string & {}` so
/// that an editor still completes the known ones.
pub(super) fn typescript_input(input: &mut Value) {
    let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) else {
        return;
    };
    for (name, open) in open_definitions(definitions) {
        let union = format!("{} | (string & {{}})", open.literals().join(" | "));
        definitions.insert(
            name,
            serde_json::json!({ "description": open.documentation(), "tsType": union }),
        );
    }
}

/// `input` with its open enumerations out of its `$defs` and every
/// reference to one naming `module`'s alias of it through
/// `datamodel-code-generator`'s `customTypePath`, since it writes a closed
/// `StrEnum` for a `oneOf` of constants. Returns them, for `module`.
pub(super) fn python_input(input: &mut Value, module: &str) -> Vec<(String, OpenValues)> {
    let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) else {
        return Vec::new();
    };
    let open = open_definitions(definitions);
    for (name, _) in &open {
        definitions.remove(name);
    }
    for (name, _) in &open {
        name_alias(
            input,
            &format!("#/$defs/{name}"),
            &format!("{module}.{name}"),
        );
    }
    open
}

/// Replaces every `$ref` to `reference` in `value` with `customTypePath`
/// naming `path`.
fn name_alias(value: &mut Value, reference: &str, path: &str) {
    match value {
        Value::Object(object) => {
            if object.get("$ref").and_then(Value::as_str) == Some(reference) {
                object.remove("$ref");
                object.insert("customTypePath".to_owned(), Value::String(path.to_owned()));
            }
            for child in object.values_mut() {
                name_alias(child, reference, path);
            }
        }
        Value::Array(items) => {
            for item in items {
                name_alias(item, reference, path);
            }
        }
        _ => {}
    }
}

fn text(object: &Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
