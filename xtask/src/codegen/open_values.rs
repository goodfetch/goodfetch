//! The open enumerations and tagged unions of the protocol (ADR 0079): a
//! type whose schema says `"open": true`, to which a newer engine may add a
//! value or a member. An open enumeration is a `snake_case` string type,
//! such as a Verdict or a Block cause: the SDKs keep its value as received,
//! so each layer types it as its known values or any other string:
//! `Known | (string & {})` in TypeScript and `Literal[...] | str` in Python,
//! never a closed union or enum. An open tagged union, such as a Stop
//! reason, gets an `unknown` member, which a member the SDK does not know
//! reads as, with the tag it was sent in `tag` and the rest as sent: adding
//! `{ kind: string }` to a TypeScript union instead would end narrowing on
//! its known members.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

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

/// An open tagged union: the field its members are told apart by, and the
/// tag of each member it knows, in order.
pub(super) struct OpenUnion {
    pub(super) tag: String,
    pub(super) known: Vec<String>,
}

/// The member a member the SDK does not know reads as, and the field that
/// keeps the tag it was sent.
const UNKNOWN: &str = "unknown";
const TAG: &str = "tag";

impl OpenUnion {
    /// `schema`'s, when it is an open tagged union: a `oneOf` of objects
    /// marked `"open": true`, each naming itself by a constant of one field
    /// they share.
    pub(super) fn of(schema: &Value) -> Option<Self> {
        let schema = schema.as_object()?;
        if schema.get(OPEN).and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let members = schema.get("oneOf")?.as_array()?;
        let tags: Vec<Vec<(&String, &str)>> = members
            .iter()
            .map(|member| {
                member
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|properties| {
                        properties
                            .iter()
                            .filter_map(|(key, property)| {
                                Some((key, property.get("const")?.as_str()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let first = tags.first()?;
        let mut shared = first.iter().map(|(key, _)| *key).filter(|key| {
            tags.iter()
                .all(|member| member.iter().any(|(other, _)| other == key))
        });
        let tag = shared.next()?;
        if shared.next().is_some() {
            return None;
        }
        let known = tags
            .iter()
            .map(|member| {
                member
                    .iter()
                    .find(|(key, _)| *key == tag)
                    .map(|(_, value)| (*value).to_owned())
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            tag: tag.clone(),
            known,
        })
    }

    /// Its `unknown` member: the tag `unknown`, the tag it was sent in
    /// `tag`, and any other field as sent; built on `base` where the layer
    /// names one.
    fn unknown_member(&self, base: Option<&str>) -> Value {
        let mut member = json!({
            "description": "A member this package does not know, which a newer engine may send: the tag it was sent in `tag`, and what else it carries as sent (ADR 0079).",
            "type": "object",
            "properties": {
                (self.tag.clone()): { "const": UNKNOWN, "type": "string" },
                TAG: { "description": "The tag the engine sent.", "type": "string" },
            },
            "required": [self.tag, TAG],
            "additionalProperties": true,
        });
        if let Some(base) = base {
            member["customBasePath"] = Value::String(base.to_owned());
        }
        member
    }
}

/// Adds its `unknown` member to each open tagged union among `input`'s
/// definitions, built on `base` where the layer names one. Refuses a schema
/// marked open that is neither an enumeration nor a tagged union, and a
/// union whose members already use the name `unknown` or the field `tag`.
pub(super) fn unknown_members(input: &mut Value, base: Option<&str>) -> Result<()> {
    let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    for (name, schema) in definitions.iter_mut() {
        if schema.get(OPEN).and_then(Value::as_bool) != Some(true)
            || OpenValues::of(schema).is_some()
        {
            continue;
        }
        let Some(union) = OpenUnion::of(schema) else {
            bail!("`{name}` is marked open but is neither an enumeration nor a tagged union");
        };
        if union.tag == TAG || union.known.iter().any(|known| known == UNKNOWN) {
            bail!("the open union `{name}` already uses the member `{UNKNOWN}` or the tag `{TAG}`");
        }
        let field = format!("/properties/{TAG}");
        if schema.pointer(&field).is_some() {
            bail!(
                "the open union `{name}` has a field `{TAG}`, which its `unknown` member keeps the tag in"
            );
        }
        let members = schema
            .get_mut("oneOf")
            .and_then(Value::as_array_mut)
            .expect("invariant: an open union has a `oneOf`");
        if members
            .iter()
            .any(|member| member.pointer(&field).is_some())
        {
            bail!(
                "a member of the open union `{name}` has a field `{TAG}`, which its `unknown` member keeps the tag in"
            );
        }
        members.push(union.unknown_member(base));
    }
    Ok(())
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
