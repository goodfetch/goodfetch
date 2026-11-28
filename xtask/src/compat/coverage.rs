//! Whether a `head/` set covers its format's whole schema (ADR 0066): every
//! property of a JSON format's schema holds a value other than `null` in at
//! least one document, or one message of a transcript, and every column of a
//! SQLite-backed format holds one in at least one row of its dumps.
//!
//! A property is named by its path from the document's root: `limits.pages`
//! for an object's property, `retries[].status` through an array's items and
//! `headers.*.name` through a map's values. The paths of every branch of
//! `oneOf`, `anyOf` and `allOf` count, so a property each branch shares is
//! covered by a document of any of them.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::registry::{self, Format, Storage};
use super::sql;
use super::tree::Tree;
use super::writer::DESCRIPTION;

fn has_extension(path: &str, extension: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(extension))
}

/// The paths of the format's schema that no file of `set` covers, sorted.
pub(super) fn uncovered(format: &Format, set: &Tree) -> Result<Vec<String>> {
    let mut uncovered = match format.storage {
        Storage::Sqlite => sql::uncovered(
            set.iter()
                .filter(|(path, _)| has_extension(path, "sql"))
                .filter_map(|(_, bytes)| std::str::from_utf8(bytes).ok()),
        ),
        Storage::Warc => warc_uncovered(set),
        Storage::Json | Storage::Transcript | Storage::Toml => {
            let mut documents = Vec::new();
            for (path, bytes) in set {
                match format.storage {
                    Storage::Json if has_extension(path, "json") && path != DESCRIPTION => {
                        documents.push(
                            serde_json::from_slice(bytes)
                                .with_context(|| format!("reading {path}"))?,
                        );
                    }
                    Storage::Toml if has_extension(path, "toml") => {
                        let text = std::str::from_utf8(bytes)
                            .with_context(|| format!("reading {path}"))?;
                        let document: toml::Table =
                            toml::from_str(text).with_context(|| format!("reading {path}"))?;
                        documents.push(
                            serde_json::to_value(document)
                                .with_context(|| format!("reading {path}"))?,
                        );
                    }
                    Storage::Transcript if has_extension(path, "jsonl") => {
                        let text = std::str::from_utf8(bytes)
                            .with_context(|| format!("reading {path}"))?;
                        for line in text.lines().filter(|line| !line.trim().is_empty()) {
                            documents.push(
                                serde_json::from_str(line)
                                    .with_context(|| format!("reading a message of {path}"))?,
                            );
                        }
                    }
                    _ => {}
                }
            }
            let schema = &format.schema;
            let mut paths = BTreeSet::new();
            properties(schema, schema, "", &mut Vec::new(), &mut paths);
            let mut covered = BTreeSet::new();
            for document in &documents {
                cover(schema, schema, document, "", &mut covered);
            }
            paths.difference(&covered).cloned().collect()
        }
    };
    uncovered.sort();
    Ok(uncovered)
}

/// Checks that a WARC profile's corpus holds both plain `.warc` and gzipped
/// `.warc.gz` files and every record kind the writer emits (`warcinfo`,
/// `request`, `response`, `revisit`, and `WARC-Truncated: disconnect`).
fn warc_uncovered(set: &Tree) -> Vec<String> {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    for (path, bytes) in set {
        if path.ends_with(".warc.gz") && bytes.starts_with(&[0x1f, 0x8b]) {
            seen.insert("gzip");
        } else if has_extension(path, "warc") {
            let text = String::from_utf8_lossy(bytes);
            for kind in ["warcinfo", "request", "response", "revisit"] {
                if text.contains(&format!("WARC-Type: {kind}")) {
                    seen.insert(kind);
                }
            }
            if text.contains("WARC-Truncated: disconnect") {
                seen.insert("truncated");
            }
        }
    }
    [
        "warcinfo",
        "request",
        "response",
        "revisit",
        "truncated",
        "gzip",
    ]
    .into_iter()
    .filter(|required| !seen.contains(required))
    .map(str::to_owned)
    .collect()
}

fn child(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

fn items(prefix: &str) -> String {
    format!("{prefix}[]")
}

fn values(prefix: &str) -> String {
    child(prefix, "*")
}

const BRANCHES: [&str; 3] = ["allOf", "anyOf", "oneOf"];

/// Every property path of `schema`, which `root` holds with its `$defs`.
/// `following` holds the types being expanded, so a recursive type ends.
fn properties(
    root: &Value,
    schema: &Value,
    prefix: &str,
    following: &mut Vec<String>,
    paths: &mut BTreeSet<String>,
) {
    let Value::Object(object) = schema else {
        return;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && !following.iter().any(|name| name == reference)
        && let Some(definition) = registry::resolve(root, reference)
    {
        following.push(reference.to_owned());
        properties(root, definition, prefix, following, paths);
        following.pop();
    }
    if let Some(Value::Object(named)) = object.get("properties") {
        for (name, property) in named {
            let path = child(prefix, name);
            paths.insert(path.clone());
            properties(root, property, &path, following, paths);
        }
    }
    for map in ["additionalProperties", "patternProperties"] {
        match object.get(map) {
            Some(Value::Object(patterns)) if map == "patternProperties" => {
                for value in patterns.values() {
                    properties(root, value, &values(prefix), following, paths);
                }
            }
            Some(value @ Value::Object(_)) => {
                properties(root, value, &values(prefix), following, paths);
            }
            _ => {}
        }
    }
    if let Some(item) = object.get("items") {
        properties(root, item, &items(prefix), following, paths);
    }
    if let Some(Value::Array(positions)) = object.get("prefixItems") {
        for item in positions {
            properties(root, item, &items(prefix), following, paths);
        }
    }
    for keyword in BRANCHES {
        if let Some(Value::Array(branches)) = object.get(keyword) {
            for branch in branches {
                properties(root, branch, prefix, following, paths);
            }
        }
    }
}

/// Every property path of `schema` that `value` gives a value other than
/// `null`.
fn cover(
    root: &Value,
    schema: &Value,
    value: &Value,
    prefix: &str,
    covered: &mut BTreeSet<String>,
) {
    let Value::Object(object) = schema else {
        return;
    };
    if value.is_null() {
        return;
    }
    if let Some(definition) = object
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| registry::resolve(root, reference))
    {
        cover(root, definition, value, prefix, covered);
    }
    for keyword in BRANCHES {
        if let Some(Value::Array(branches)) = object.get(keyword) {
            for branch in branches {
                cover(root, branch, value, prefix, covered);
            }
        }
    }
    match value {
        Value::Object(fields) => {
            let named = object.get("properties").and_then(Value::as_object);
            for (name, field) in fields {
                if let Some(property) = named.and_then(|named| named.get(name)) {
                    if !field.is_null() {
                        let path = child(prefix, name);
                        covered.insert(path.clone());
                        cover(root, property, field, &path, covered);
                    }
                    continue;
                }
                if let Some(additional @ Value::Object(_)) = object.get("additionalProperties") {
                    cover(root, additional, field, &values(prefix), covered);
                }
                if let Some(Value::Object(patterns)) = object.get("patternProperties") {
                    for pattern in patterns.values() {
                        cover(root, pattern, field, &values(prefix), covered);
                    }
                }
            }
        }
        Value::Array(elements) => {
            let positions = object.get("prefixItems").and_then(Value::as_array);
            for (position, element) in elements.iter().enumerate() {
                if let Some(item) = positions.and_then(|positions| positions.get(position)) {
                    cover(root, item, element, &items(prefix), covered);
                } else if let Some(item) = object.get("items") {
                    cover(root, item, element, &items(prefix), covered);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::super::registry;
    use super::super::tree::Tree;
    use super::uncovered;

    /// A bundle registering one JSON format whose schema is `Root`.
    fn bundle(definitions: &serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "formats": [{
                "name": "sample",
                "version": "0.1",
                "storage": "json",
                "schema": { "$ref": "#/$defs/Root" },
            }],
            "$defs": definitions,
        }))
        .unwrap()
    }

    #[rstest]
    #[case::recursive_type_ends_at_its_first_repeat(
        json!({
            "Root": { "$ref": "#/$defs/Node" },
            "Node": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "children": { "type": "array", "items": { "$ref": "#/$defs/Node" } },
                },
            },
        }),
        json!({ "name": "a", "children": [{ "name": "b", "children": [] }] }),
        vec![],
    )]
    #[case::map_values_through_pattern_properties(
        json!({
            "Root": {
                "type": "object",
                "properties": {
                    "hosts": {
                        "type": "object",
                        "patternProperties": {
                            "^[a-z.]+$": {
                                "type": "object",
                                "properties": { "delay": { "type": "integer" } },
                            },
                        },
                    },
                },
            },
        }),
        json!({ "hosts": { "example.invalid": {} } }),
        vec!["hosts.*.delay"],
    )]
    #[case::tuple_positions(
        json!({
            "Root": {
                "type": "object",
                "properties": {
                    "pair": {
                        "type": "array",
                        "prefixItems": [
                            { "type": "object", "properties": { "left": { "type": "string" } } },
                            { "type": "object", "properties": { "right": { "type": "string" } } },
                        ],
                    },
                },
            },
        }),
        json!({ "pair": [{ "left": "a" }] }),
        vec!["pair[].right"],
    )]
    fn a_property_is_uncovered_without_a_value_in_any_document(
        #[case] definitions: serde_json::Value,
        #[case] document: serde_json::Value,
        #[case] expected: Vec<&str>,
    ) {
        let format = registry::parse(&bundle(&definitions))
            .unwrap()
            .pop()
            .unwrap();
        let set = Tree::from([(
            "document.json".to_owned(),
            serde_json::to_vec(&document).unwrap(),
        )]);

        let uncovered = uncovered(&format, &set).unwrap();

        assert_eq!(uncovered, expected);
    }

    #[test]
    fn a_toml_document_covers_the_properties_it_holds() {
        let text = serde_json::to_vec(&json!({
            "formats": [{
                "name": "settings",
                "version": "0.1",
                "storage": "toml",
                "schema": {
                    "type": "object",
                    "properties": {
                        "version": { "type": "string" },
                        "settings": {
                            "type": "object",
                            "properties": {
                                "network": {
                                    "type": "object",
                                    "properties": {
                                        "offline": { "type": "boolean" },
                                        "resolve": { "type": "object" },
                                    },
                                },
                            },
                        },
                    },
                },
            }],
        }))
        .unwrap();
        let format = registry::parse(&text).unwrap().pop().unwrap();
        let set = Tree::from([(
            "policy.toml".to_owned(),
            b"version = \"0.1\"\n[settings.network]\noffline = true\n".to_vec(),
        )]);

        let uncovered = uncovered(&format, &set).unwrap();

        assert_eq!(uncovered, ["settings.network.resolve"]);
    }
}
