//! The format registry, read from a JSON Schema bundle (ADR 0066): each
//! registered format's name, version, storage and schema. `goodfetch-protocol`
//! defines the registry and `xtask codegen` writes it into the bundle as
//! `formats`; a bundle without it registers no format.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::releases::{FormatName, FormatVersion};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const DEFINITIONS: &str = "#/$defs/";

/// How a format's files are stored in its corpus sets (ADR 0066).
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Storage {
    /// JSON documents, each a `.json` file.
    Json,
    /// A SQLite-backed store, as SQL text dumps in `.sql` files.
    Sqlite,
    /// A protocol's recorded transcripts: `.jsonl` files of one message per line.
    Transcript,
    /// TOML documents, each a `.toml` file, such as settings files.
    Toml,
    /// WARC 1.1 files (`.warc` and `.warc.gz`), plain and gzipped per record.
    Warc,
}

impl Storage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Sqlite => "sqlite",
            Self::Transcript => "transcript",
            Self::Toml => "toml",
            Self::Warc => "warc",
        }
    }
}

/// One registered format.
pub(super) struct Format {
    pub(super) name: FormatName,
    pub(super) version: FormatVersion,
    pub(super) storage: Storage,
    /// Its schema, standing alone: the registry's schema with every type of
    /// the bundle it reaches under its own `$defs`.
    pub(super) schema: Value,
}

impl Format {
    /// The file `xtask compat` adds to each set: the format's name, version,
    /// storage and schema, which a frozen set keeps and the patch check
    /// compares.
    pub(super) fn description(&self) -> Value {
        json!({
            "name": self.name.as_str(),
            "version": self.version.to_string(),
            "storage": self.storage.as_str(),
            "schema": self.schema,
        })
    }
}

#[derive(Deserialize)]
struct Bundle {
    #[serde(default)]
    formats: Vec<Entry>,
    #[serde(rename = "$defs", default)]
    definitions: Map<String, Value>,
}

#[derive(Deserialize)]
struct Entry {
    name: FormatName,
    version: FormatVersion,
    storage: Storage,
    schema: Value,
}

/// The formats the bundle's text registers, sorted by name.
pub(super) fn parse(text: &[u8]) -> Result<Vec<Format>> {
    let bundle: Bundle = serde_json::from_slice(text).context("reading the format registry")?;
    let mut names = BTreeSet::new();
    let mut formats = Vec::new();
    for entry in bundle.formats {
        if !names.insert(entry.name.clone()) {
            bail!("the format `{}` is registered twice", entry.name);
        }
        let schema = standalone(entry.schema, &bundle.definitions)
            .with_context(|| format!("reading the schema of `{}`", entry.name))?;
        formats.push(Format {
            name: entry.name,
            version: entry.version,
            storage: entry.storage,
            schema,
        });
    }
    formats.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(formats)
}

/// `schema` standing alone: with the bundle's `definitions` it reaches under
/// its own `$defs`, and none when it reaches none.
pub(crate) fn standalone(mut schema: Value, definitions: &Map<String, Value>) -> Result<Value> {
    let reached = reached(&schema, definitions)?;
    if !reached.is_empty()
        && let Value::Object(object) = &mut schema
    {
        object.insert(
            "$defs".to_owned(),
            Value::Object(reached.into_iter().collect()),
        );
    }
    Ok(schema)
}

/// Every type of the bundle that `schema` refers to, directly or through
/// other types.
pub(crate) fn reached(
    schema: &Value,
    definitions: &Map<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    let mut reached = BTreeMap::new();
    let mut pending = references(schema);
    while let Some(name) = pending.pop() {
        if reached.contains_key(&name) {
            continue;
        }
        let Some(definition) = definitions.get(&name) else {
            bail!("`{DEFINITIONS}{name}` is no type of the bundle");
        };
        pending.extend(references(definition));
        reached.insert(name, definition.clone());
    }
    Ok(reached)
}

/// The type names of every `$ref` in `value`.
fn references(value: &Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = vec![value];
    while let Some(current) = pending.pop() {
        match current {
            Value::Object(object) => {
                if let Some(name) = object
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|reference| reference.strip_prefix(DEFINITIONS))
                {
                    names.push(name.to_owned());
                }
                pending.extend(object.values());
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    names
}

/// The type a `$ref` of a standing-alone schema names, from its `$defs`.
pub(super) fn resolve<'schema>(root: &'schema Value, reference: &str) -> Option<&'schema Value> {
    let name = reference.strip_prefix(DEFINITIONS)?;
    root.get("$defs")?.get(name)
}

/// Keywords that annotate a schema without changing what it accepts.
const ANNOTATIONS: [&str; 6] = [
    "description",
    "title",
    "examples",
    "$comment",
    "deprecated",
    "x-goodfetch-personal",
];

/// Keywords whose value is a schema.
const SCHEMA_KEYWORDS: [&str; 12] = [
    "additionalProperties",
    "items",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
    "unevaluatedProperties",
    "unevaluatedItems",
    "additionalItems",
    "contentSchema",
];

/// Keywords whose value maps names to schemas.
const SCHEMA_MAPS: [&str; 4] = [
    "properties",
    "patternProperties",
    "$defs",
    "dependentSchemas",
];

/// Keywords whose value is a list of schemas.
const SCHEMA_LISTS: [&str; 4] = ["allOf", "anyOf", "oneOf", "prefixItems"];

/// The schema without its annotations, so that rewording a description is
/// no change of the format.
pub(super) fn without_annotations(schema: &Value) -> Value {
    let Value::Object(object) = schema else {
        return schema.clone();
    };
    let mut stripped = Map::new();
    for (keyword, value) in object {
        if ANNOTATIONS.contains(&keyword.as_str()) {
            continue;
        }
        let value = if SCHEMA_KEYWORDS.contains(&keyword.as_str()) {
            without_annotations(value)
        } else if SCHEMA_MAPS.contains(&keyword.as_str()) {
            match value {
                Value::Object(map) => Value::Object(
                    map.iter()
                        .map(|(name, schema)| (name.clone(), without_annotations(schema)))
                        .collect(),
                ),
                other => other.clone(),
            }
        } else if SCHEMA_LISTS.contains(&keyword.as_str()) {
            match value {
                Value::Array(schemas) => {
                    Value::Array(schemas.iter().map(without_annotations).collect())
                }
                other => other.clone(),
            }
        } else {
            value.clone()
        };
        stripped.insert(keyword.clone(), value);
    }
    Value::Object(stripped)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::without_annotations;

    /// `"x-goodfetch-personal"` is treated as a schema annotation that does
    /// not alter what a format accepts (ADR 0016, ticket 673).
    ///
    /// Covers: story:4-78
    #[test]
    fn without_annotations_strips_personal_data_keyword() {
        let annotated = json!({
            "type": "object",
            "properties": {
                "email": { "type": "string", "x-goodfetch-personal": true }
            }
        });
        let plain = json!({
            "type": "object",
            "properties": {
                "email": { "type": "string" }
            }
        });

        assert_eq!(without_annotations(&annotated), plain);
    }
}
