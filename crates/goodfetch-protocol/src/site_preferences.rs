//! What a run declares its results are for, its Purpose, and whether the
//! AI-use signals a site sends are applied to it (ADRs 0016, 0045 and
//! 0086): settings of every run of either Product. The signals are read on
//! every run; only with `site_preferences.respect_use_signals` on does a
//! crawl skip a request whose signal refuses the Purpose, with
//! `use_signals`. A Purpose left at `search` is no declaration to judge
//! them against. Whether a program's own navigation reads robots.txt for
//! Provenance is a setting too, off by default (ticket #2448).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::catalog::{
    Authority, Entries, ProtocolVersion, Setting, SharedType, SharedTypeValue, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The JSON Schema annotation keyword that marks a field as holding personal
/// data (`"x-goodfetch-personal": true`, ADR 0016): advisory only, showing the
/// Run's personal-data notice once and recording it in the Audit log without
/// blocking or delaying the Run.
pub const PERSONAL_DATA_KEYWORD: &str = "x-goodfetch-personal";

/// A Run whose schema marked one or more fields as personal showed ADR 0016's
/// personal-data notice once.
pub const PERSONAL_DATA_NOTICE: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "personal_data_notice",
    description: "A Run's schema marked one or more fields as personal (`\"x-goodfetch-personal\": true`): the advisory personal-data notice was shown once for the Run, naming the fields, without blocking it (ADR 0016).",
    arrived_in: ARRIVED_IN,
};

/// Maximum recursion depth when walking a caller-supplied JSON Schema.
const MAX_SCHEMA_DEPTH: usize = 64;

/// Maximum schema nodes visited per call, bounding work on pathological schemas.
const MAX_SCHEMA_NODES: usize = 10_000;

/// The field paths in `schema` marked with `"x-goodfetch-personal": true`
/// (ADR 0016), sorted and deduplicated.
///
/// Walks object `properties` (joining nested names with `.`), array `items`
/// and `prefixItems` (`[]`), `additionalProperties`, schema combinators
/// (`allOf`, `anyOf`, `oneOf`, `if`, `then`, `else`), and local `$ref` pointers
/// into `$defs` or `definitions`. Memoizes each `$ref` target's relative
/// personal-field paths and caps recursion depth and total visited nodes so
/// cyclic or diamond `$ref` DAGs stay linear and bounded.
#[must_use]
pub fn personal_fields(schema: &Value) -> Vec<String> {
    let mut walker = PersonalWalker {
        root: schema,
        visiting: BTreeSet::new(),
        memo: BTreeMap::new(),
        visited: 0,
    };
    let mut found = BTreeSet::new();
    walker.collect(schema, "", 0, &mut found);
    found.into_iter().collect()
}

struct PersonalWalker<'schema> {
    root: &'schema Value,
    visiting: BTreeSet<String>,
    memo: BTreeMap<String, Vec<String>>,
    visited: usize,
}

impl<'schema> PersonalWalker<'schema> {
    fn collect(
        &mut self,
        node: &'schema Value,
        path: &str,
        depth: usize,
        found: &mut BTreeSet<String>,
    ) {
        if depth > MAX_SCHEMA_DEPTH || self.visited >= MAX_SCHEMA_NODES {
            return;
        }
        self.visited += 1;
        let Some(object) = node.as_object() else {
            return;
        };
        if object.get(PERSONAL_DATA_KEYWORD) == Some(&Value::Bool(true)) && !path.is_empty() {
            found.insert(path.to_owned());
        }
        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            self.collect_ref(reference, path, depth, found);
        }
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            for (name, property) in properties {
                let child = join_path(path, name);
                self.collect(property, &child, depth + 1, found);
            }
        }
        if let Some(items) = object.get("items") {
            let item_path = join_array(path);
            self.collect(items, &item_path, depth + 1, found);
        }
        if let Some(prefix_items) = object.get("prefixItems").and_then(Value::as_array) {
            let item_path = join_array(path);
            for item in prefix_items {
                self.collect(item, &item_path, depth + 1, found);
            }
        }
        if let Some(additional) = object.get("additionalProperties") {
            self.collect(additional, path, depth + 1, found);
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = object.get(keyword).and_then(Value::as_array) {
                for branch in branches {
                    self.collect(branch, path, depth + 1, found);
                }
            }
        }
        for keyword in ["if", "then", "else"] {
            if let Some(branch) = object.get(keyword) {
                self.collect(branch, path, depth + 1, found);
            }
        }
    }

    fn collect_ref(
        &mut self,
        reference: &str,
        path: &str,
        depth: usize,
        found: &mut BTreeSet<String>,
    ) {
        if let Some(relative) = self.memo.get(reference) {
            for suffix in relative {
                let combined = combine_relative(path, suffix);
                if !combined.is_empty() {
                    found.insert(combined);
                }
            }
            return;
        }
        let Some(target) = resolve_local_ref(self.root, reference) else {
            return;
        };
        if !self.visiting.insert(reference.to_owned()) {
            return;
        }
        let mut relative_set = BTreeSet::new();
        if target
            .as_object()
            .and_then(|obj| obj.get(PERSONAL_DATA_KEYWORD))
            == Some(&Value::Bool(true))
        {
            relative_set.insert(String::new());
        }
        self.collect(target, "", depth + 1, &mut relative_set);
        self.visiting.remove(reference);
        let relative: Vec<String> = relative_set.into_iter().collect();
        for suffix in &relative {
            let combined = combine_relative(path, suffix);
            if !combined.is_empty() {
                found.insert(combined);
            }
        }
        // Only memoize once no active cycle could have truncated this definition.
        if self.visiting.is_empty() {
            self.memo.insert(reference.to_owned(), relative);
        }
    }
}

fn join_path(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_owned()
    } else {
        format!("{prefix}.{segment}")
    }
}

fn join_array(prefix: &str) -> String {
    if prefix.is_empty() {
        "[]".to_owned()
    } else {
        format!("{prefix}[]")
    }
}

fn combine_relative(prefix: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        prefix.to_owned()
    } else if prefix.is_empty() {
        suffix.to_owned()
    } else if suffix.starts_with("[]") {
        format!("{prefix}{suffix}")
    } else {
        format!("{prefix}.{suffix}")
    }
}

fn resolve_local_ref<'schema>(root: &'schema Value, reference: &str) -> Option<&'schema Value> {
    if let Some(name) = reference.strip_prefix("#/$defs/") {
        return root.get("$defs")?.get(name);
    }
    if let Some(name) = reference.strip_prefix("#/definitions/") {
        return root.get("definitions")?.get(name);
    }
    None
}

/// `site_preferences.purpose`'s values, in the catalog's order (ADR 0016).
pub const PURPOSE_VALUES: &[&str] = &["search", "ai-input", "ai-train", "collect"];

fn search() -> Value {
    Value::String("search".to_owned())
}

fn no() -> Value {
    Value::Bool(false)
}

/// `site_preferences.purpose` (ADRs 0016 and 0086).
pub const SITE_PREFERENCES_PURPOSE: Setting = Setting {
    name: "site_preferences.purpose",
    description: "What the run's results are for, against which the AI-use signals are judged while `site_preferences.respect_use_signals` is on: `search` (the default, which no signal is judged against), `ai-input` (input to an AI model), `ai-train` (training an AI model) or `collect` (plain collection, text and data mining).",
    kind: ValueKind::Choice(PURPOSE_VALUES),
    default: search,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `site_preferences.respect_use_signals` (ADRs 0016, 0045 and 0086).
pub const SITE_PREFERENCES_RESPECT_USE_SIGNALS: Setting = Setting {
    name: "site_preferences.respect_use_signals",
    description: "Whether the AI-use signals a site sends (IETF aipref's `Content-Usage` header, TDMRep's `tdm-reservation` header) are applied to the run's `site_preferences.purpose`: on, a crawl request whose response's signal refuses the Purpose ends `skipped` with `use_signals` and its handler never runs. Off by default; the signals are read either way.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `site_preferences.read_robots_txt_for_navigation` (ADRs 0016, 0086 and
/// 0138).
pub const SITE_PREFERENCES_READ_ROBOTS_TXT_FOR_NAVIGATION: Setting = Setting {
    name: "site_preferences.read_robots_txt_for_navigation",
    description: "Whether a page a program opened reads its origin's robots.txt for Provenance: on, each document its main frame receives and each `page.request` it sends reads the origin's robots.txt once per Run, beside the navigation, and Provenance records whether the page was allowed, never gating it. Off by default: nothing is fetched and no Provenance entry is written. A crawl's own requests and a handler's navigations read and obey robots.txt either way, under Foxhound's `site_preferences.respect_robots_txt`.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        SITE_PREFERENCES_PURPOSE,
        SITE_PREFERENCES_RESPECT_USE_SIGNALS,
        SITE_PREFERENCES_READ_ROBOTS_TXT_FOR_NAVIGATION,
    ],
    hook_stages: &[],
    shared_type_values: &[PERSONAL_DATA_NOTICE],
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::personal_fields;

    #[test]
    fn personal_fields_finds_top_level_nested_array_and_referenced_fields_only_when_true() {
        let schema = json!({
            "type": "object",
            "$defs": {
                "Contact": {
                    "type": "object",
                    "properties": {
                        "phone": { "type": "string", "x-goodfetch-personal": true },
                        "self": { "$ref": "#/$defs/Contact" }
                    }
                }
            },
            "properties": {
                "email": { "type": "string", "x-goodfetch-personal": true },
                "public_name": { "type": "string", "x-goodfetch-personal": false },
                "title": { "type": "string" },
                "contact": { "$ref": "#/$defs/Contact" },
                "attendees": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "full_name": { "type": "string", "x-goodfetch-personal": true }
                        }
                    }
                }
            }
        });

        assert_eq!(
            personal_fields(&schema),
            vec!["attendees[].full_name", "contact.phone", "email"]
        );
    }

    #[test]
    fn personal_fields_bounds_diamond_ref_dags() {
        let mut defs = serde_json::Map::new();
        for level in 0..30 {
            let next = format!("#/$defs/D{}", level + 1);
            defs.insert(
                format!("D{level}"),
                json!({
                    "type": "object",
                    "properties": {
                        "a": { "$ref": next },
                        "b": { "$ref": next },
                    }
                }),
            );
        }
        defs.insert(
            "D30".to_owned(),
            json!({
                "type": "object",
                "properties": {
                    "email": { "type": "string", "x-goodfetch-personal": true }
                }
            }),
        );
        let schema = json!({
            "type": "object",
            "$defs": defs,
            "properties": { "root": { "$ref": "#/$defs/D0" } }
        });

        let found = personal_fields(&schema);
        assert!(
            found
                .iter()
                .any(|path| path.rsplit('.').next() == Some("email")),
            "expected bounded traversal to find email: {found:?}"
        );
    }
}
