//! The API breakage check of the contracts (ADRs 0030, 0050, 0069, 0094 and
//! 0095): each Product's catalog, read from its projection (operations,
//! events, settings, metric names, error and exit codes and the rest), and
//! the bundle's `--json` output types, hand-written commands' included,
//! compared with the same files at the commit `compat/api.toml` names as the
//! previous release. Before the first release that commit is one of `main`'s
//! that stands for it (issue 148); a release's pull request moves it to the
//! release's own.
//!
//! Each Product is compared with its own projection, so an entry that leaves
//! Collie's catalog for Foxhound's is a break for Collie alone, and one that
//! joins Collie's from Foxhound's an addition (ADR 0095).
//!
//! A change is a break or an addition by the side its data flows. What a
//! user sends (an operation's parameters, a setting's value, a handler's
//! answer) may accept more and never less; what Goodfetch answers (results,
//! event payloads, error details, `--json` output) may say more, new values
//! and fields included, as the SDKs keep what they do not know (ADRs 0079
//! and 0082), and never less. Descriptions, the version an entry arrived in
//! and its deprecation change nothing. A changed default is a break only
//! when it weakens a guarantee (ADR 0030): when it moves toward the values
//! that weaken one, as the previous release's `weakening` says, and, where
//! that says no direction, whenever it changes ([`defaults`]). Anything else that
//! changed, and that the rules cannot tell apart, is a break.
//!
//! The Collie client packages carry, beside Collie's projection, the
//! bundle's `connection` section, which is compared too ([`connection`]);
//! their code, generated and hand-written, is `xtask sdk-api`'s and `xtask
//! semver-checks`'.
//!
//! An entry that was Experimental may change in any way, and a break that
//! `compat/api.toml` announces for the next major passes; each other break
//! fails, one finding per entry. An announcement must name a break the
//! comparison finds, so the list stays the one the release's pull request
//! shows the owner. Additions and announced breaks are listed as notes.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::process::ExitCode;
use std::sync::LazyLock;

use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Value};

use crate::api_baseline::{self, Changes, Outcome};
use crate::check::Finding;
use crate::git;

mod connection;
mod defaults;

/// Each Product's projection of the catalog.
const PROJECTIONS: [(&str, &str); 2] = [
    ("collie", "crates/goodfetch-protocol/generated/collie.json"),
    (
        "foxhound",
        "crates/goodfetch-protocol/generated/foxhound.json",
    ),
];

/// The bundle, whose `outputs` are the `--json` output types and
/// `command_failed` that of every command that fails (ADR 0069).
const BUNDLE: &str = "crates/goodfetch-protocol/generated/bundle.json";

/// An entry's fields that no caller depends on.
const UNCHECKED_FIELDS: [&str; 5] = [
    "arrived_in",
    "deprecated",
    "description",
    "product",
    "products",
];

/// The schema keywords that describe and validate nothing.
const UNCHECKED_KEYWORDS: [&str; 10] = [
    "$comment",
    "arrived_in",
    "default",
    "deprecated",
    "description",
    "examples",
    "readOnly",
    "title",
    "writeOnly",
    "$schema",
];

/// The keywords the comparison reads itself.
const COMPARED_KEYWORDS: [&str; 10] = [
    "$ref",
    "additionalProperties",
    "anyOf",
    "const",
    "enum",
    "items",
    "oneOf",
    "properties",
    "required",
    "type",
];

/// The bounds a value may be held to: a lower one may only fall, an upper
/// one only rise, for what a user sends to accept more.
const LOWER_BOUNDS: [&str; 5] = [
    "exclusiveMinimum",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
];
const UPPER_BOUNDS: [&str; 5] = [
    "exclusiveMaximum",
    "maxItems",
    "maxLength",
    "maxProperties",
    "maximum",
];

/// Which side of the contract writes the data a schema describes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    /// The user sends it and Goodfetch reads it: it may accept more.
    Input,
    /// Goodfetch sends it and the user reads it: it may say more.
    Output,
    /// Both peers send it and read it, as a Connection's `$/cancel`: it may
    /// neither narrow nor widen.
    Both,
}

/// The contracts' changes by the side their data flows.
impl Changes {
    /// `breaks` on the side that may not narrow, `adds` on the other.
    fn narrows(&mut self, side: Side, at: &str, what: impl std::fmt::Display) {
        match side {
            Side::Input | Side::Both => self.breaks(at, what),
            Side::Output => self.adds(at, what),
        }
    }

    /// `breaks` on the side that may not widen, `adds` on the other.
    fn widens(&mut self, side: Side, at: &str, what: impl std::fmt::Display) {
        match side {
            Side::Input => self.adds(at, what),
            Side::Output | Side::Both => self.breaks(at, what),
        }
    }
}

pub(crate) fn run(root: &Path) -> Result<ExitCode> {
    Ok(check(root)?.report())
}

fn check(root: &Path) -> Result<Outcome> {
    let baseline = api_baseline::baseline(root)?;
    let commit = &baseline.commit;
    let mut entries: BTreeMap<String, Changes> = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for (product, path) in PROJECTIONS {
        let previous = previous_json(root, commit, path)?;
        let current = current_json(root, path)?;
        compare_projection(product, &previous, &current, &mut entries, &mut duplicates)?;
    }
    let previous = previous_json(root, commit, BUNDLE)?;
    let current = current_json(root, BUNDLE)?;
    compare_outputs(&previous, &current, &mut entries);
    connection::compare(&previous, &current, &mut entries);

    let mut outcome = baseline.verdict(|entry| !api_baseline::is_package_break(entry), &entries);
    outcome.findings.extend(duplicates.iter().map(|entry| {
        Finding::new(
            "duplicate-entry",
            entry,
            "two entries share this key, so the comparison would read one of them only",
        )
    }));
    Ok(outcome)
}

/// `path` at `commit`, a commit's full name, which no option starts with.
fn previous_json(root: &Path, commit: &str, path: &str) -> Result<Value> {
    let bytes = git::bytes(root, &["show", &format!("{commit}:{path}")])
        .with_context(|| format!("reading {path} at the previous release's commit {commit}"))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {path} at {commit}"))
}

fn current_json(root: &Path, path: &str) -> Result<Value> {
    let text =
        std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {path}"))
}

/// Compares one Product's projections section by section, each entry by its
/// name.
fn compare_projection(
    product: &str,
    previous: &Value,
    current: &Value,
    entries: &mut BTreeMap<String, Changes>,
    duplicates: &mut BTreeSet<String>,
) -> Result<()> {
    let definitions = Definitions::of(previous, current);
    let Some(sections) = previous.as_object() else {
        bail!("{product}'s projection is no object");
    };
    for (section, previous_entries) in sections {
        let Some(previous_entries) = previous_entries.as_array() else {
            continue;
        };
        let previous_entries = keyed(product, section, previous_entries, duplicates);
        let current_entries = current
            .get(section)
            .and_then(Value::as_array)
            .map(|entries| keyed(product, section, entries, duplicates))
            .unwrap_or_default();
        for (name, previous_entry) in &previous_entries {
            if is_experimental(previous_entry) {
                continue;
            }
            let subject = format!("{product} {section} {name}");
            let changes = entries.entry(subject).or_default();
            match current_entries.get(name) {
                None => changes.breaks(name, "removed"),
                Some(current_entry) => {
                    compare_entry(
                        section,
                        previous_entry,
                        current_entry,
                        &definitions,
                        changes,
                    );
                }
            }
        }
        for name in current_entries
            .keys()
            .filter(|name| !previous_entries.contains_key(*name))
        {
            entries
                .entry(format!("{product} {section} {name}"))
                .or_default()
                .adds(name, "added");
        }
    }
    entries.retain(|_, changes| !changes.is_empty());
    Ok(())
}

/// A section's entries by their key: the name, which a shared type's value
/// shares with the values of other types, so its key is `<type>.<name>`.
/// Two entries under one key are set aside in `duplicates`, as the
/// comparison would read one of them only.
fn keyed<'a>(
    product: &str,
    section: &str,
    entries: &'a [Value],
    duplicates: &mut BTreeSet<String>,
) -> BTreeMap<String, &'a Value> {
    let mut keyed = BTreeMap::new();
    for entry in entries {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let key = match (section, entry.get("type").and_then(Value::as_str)) {
            ("shared_type_values", Some(kind)) => format!("{kind}.{name}"),
            _ => name.to_owned(),
        };
        if keyed.insert(key.clone(), entry).is_some() {
            duplicates.insert(format!("{product} {section} {key}"));
        }
    }
    keyed
}

fn by_name(entries: &[Value]) -> BTreeMap<String, &Value> {
    entries
        .iter()
        .filter_map(|entry| Some((entry.get("name")?.as_str()?.to_owned(), entry)))
        .collect()
}

fn is_experimental(entry: &Value) -> bool {
    entry.get("stability").and_then(Value::as_str) == Some("experimental")
}

/// The side each schema-valued field of an entry describes.
fn schema_side(section: &str, field: &str) -> Option<Side> {
    match (section, field) {
        ("operations", "params") | ("settings", "value") | ("handler_requests", "result") => {
            Some(Side::Input)
        }
        ("operations", "result")
        | ("events", "payload")
        | ("handler_requests", "params")
        | ("error_codes", "details")
        | ("kind_parts", "type") => Some(Side::Output),
        _ => None,
    }
}

fn compare_entry(
    section: &str,
    previous: &Value,
    current: &Value,
    definitions: &Definitions<'_>,
    changes: &mut Changes,
) {
    let (Some(previous), Some(current)) = (previous.as_object(), current.as_object()) else {
        if previous != current {
            changes.breaks("entry", "changed");
        }
        return;
    };
    let fields: BTreeSet<&String> = previous.keys().chain(current.keys()).collect();
    for field in fields {
        if UNCHECKED_FIELDS.contains(&field.as_str()) {
            continue;
        }
        let before = previous.get(field).unwrap_or(&Value::Null);
        let after = current.get(field).unwrap_or(&Value::Null);
        if let Some(side) = schema_side(section, field) {
            Schemas::new(definitions).compare(before, after, side, field, changes);
            continue;
        }
        if before == after {
            continue;
        }
        match field.as_str() {
            "stability" => changes.breaks(field, format!("made {after} from {before}")),
            "default" if section == "settings" => {
                defaults::compare(previous, current, before, after, changes);
            }
            // A release from before the projection carried the direction
            // published none: the field's arrival changes no promise.
            "weakening" if section == "settings" && !previous.contains_key(field) => {}
            "cancellable" | "clearable" if after == &Value::Bool(true) => {
                changes.adds(field, "now true");
            }
            "methods" => compare_methods(before, after, definitions, changes),
            _ => compare_plain(field, before, after, changes),
        }
    }
}

/// A field that no schema describes. A list of strings, or of named objects
/// such as a metric's labels, may gain members and never lose one, and a
/// named object's own fields follow the same rule; anything else that
/// changed is a break.
fn compare_plain(at: &str, before: &Value, after: &Value, changes: &mut Changes) {
    if before == after {
        return;
    }
    if let (Some(before), Some(after)) = (string_set(before), string_set(after)) {
        for removed in before.difference(&after) {
            changes.breaks(at, format!("`{removed}` removed"));
        }
        for added in after.difference(&before) {
            changes.adds(at, format!("`{added}` added"));
        }
        return;
    }
    if let (Some(before), Some(after)) = (named(before), named(after)) {
        for (name, previous) in &before {
            let at = format!("{at}.{name}");
            match after.get(name) {
                None => changes.breaks(&at, "removed"),
                Some(current) => {
                    let fields: BTreeSet<&String> = [previous, current]
                        .into_iter()
                        .filter_map(|member| member.as_object())
                        .flat_map(Map::keys)
                        .collect();
                    for field in fields {
                        compare_plain(
                            &format!("{at}.{field}"),
                            previous.get(field).unwrap_or(&Value::Null),
                            current.get(field).unwrap_or(&Value::Null),
                            changes,
                        );
                    }
                }
            }
        }
        for name in after.keys().filter(|name| !before.contains_key(*name)) {
            changes.adds(&format!("{at}.{name}"), "added");
        }
        return;
    }
    changes.breaks(at, format!("changed from {before} to {after}"));
}

/// A list whose members are all objects with a name, by that name.
fn named(value: &Value) -> Option<BTreeMap<String, &Value>> {
    value
        .as_array()?
        .iter()
        .map(|member| Some((member.get("name")?.as_str()?.to_owned(), member)))
        .collect()
}

/// A Plugin kind's methods, which a plugin implements: it reads their
/// parameters and writes their results, and a method added or removed
/// breaks the plugins written for the kind.
fn compare_methods(
    before: &Value,
    after: &Value,
    definitions: &Definitions<'_>,
    changes: &mut Changes,
) {
    let (before, after) = (methods(before), methods(after));
    for name in before.keys().filter(|name| !after.contains_key(*name)) {
        changes.breaks(&format!("methods.{name}"), "removed");
    }
    for name in after.keys().filter(|name| !before.contains_key(*name)) {
        changes.breaks(
            &format!("methods.{name}"),
            "added, which every plugin of the kind must now implement",
        );
    }
    for (name, previous) in &before {
        let Some(current) = after.get(name) else {
            continue;
        };
        for (field, side) in [("params", Side::Output), ("result", Side::Input)] {
            Schemas::new(definitions).compare(
                previous.get(field).unwrap_or(&Value::Null),
                current.get(field).unwrap_or(&Value::Null),
                side,
                &format!("methods.{name}.{field}"),
                changes,
            );
        }
    }
}

fn methods(value: &Value) -> BTreeMap<String, &Value> {
    value
        .as_array()
        .map(|methods| by_name(methods))
        .unwrap_or_default()
}

fn string_set(value: &Value) -> Option<BTreeSet<&str>> {
    value.as_array()?.iter().map(Value::as_str).collect()
}

/// Compares the bundle's `--json` output types: each command's, and
/// `command_failed`, that of every command that fails.
fn compare_outputs(previous: &Value, current: &Value, entries: &mut BTreeMap<String, Changes>) {
    let definitions = Definitions::of(previous, current);
    let outputs = |bundle: &Value| -> BTreeMap<String, Value> {
        let commands = bundle
            .get("outputs")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(command, schema)| (format!("outputs {command}"), schema.clone()));
        let failure = bundle
            .get("command_failed")
            .map(|schema| ("command_failed".to_owned(), schema.clone()));
        commands.chain(failure).collect()
    };
    let (before, after) = (outputs(previous), outputs(current));
    for (output, previous_schema) in &before {
        let mut changes = Changes::default();
        match after.get(output) {
            None => changes.breaks(output, "removed"),
            Some(current_schema) => {
                Schemas::new(&definitions).compare(
                    previous_schema,
                    current_schema,
                    Side::Output,
                    output,
                    &mut changes,
                );
            }
        }
        if !changes.is_empty() {
            entries.insert(format!("bundle {output}"), changes);
        }
    }
    for output in after.keys().filter(|output| !before.contains_key(*output)) {
        entries
            .entry(format!("bundle {output}"))
            .or_default()
            .adds(output, "added");
    }
}

/// The `$defs` each side's references point into.
struct Definitions<'a> {
    previous: &'a Map<String, Value>,
    current: &'a Map<String, Value>,
}

impl<'a> Definitions<'a> {
    /// The `$defs` of a previous and a current file, empty where one has none.
    fn of(previous: &'a Value, current: &'a Value) -> Self {
        static NONE: LazyLock<Map<String, Value>> = LazyLock::new(Map::new);
        let definitions = |file: &'a Value| {
            file.get("$defs")
                .and_then(Value::as_object)
                .unwrap_or(&NONE)
        };
        Self {
            previous: definitions(previous),
            current: definitions(current),
        }
    }
}

/// A walk over a pair of schemas, which follows each side's references and
/// compares a pair of definitions once per side, so a recursive type ends.
struct Schemas<'a> {
    definitions: &'a Definitions<'a>,
    compared: HashSet<(String, String, Side)>,
}

impl<'a> Schemas<'a> {
    fn new(definitions: &'a Definitions<'a>) -> Self {
        Self {
            definitions,
            compared: HashSet::new(),
        }
    }

    fn compare(
        &mut self,
        previous: &Value,
        current: &Value,
        side: Side,
        at: &str,
        changes: &mut Changes,
    ) {
        let previous_reference = reference(previous);
        let current_reference = reference(current);
        if let (Some(before), Some(after)) = (previous_reference, current_reference)
            && !self
                .compared
                .insert((before.to_owned(), after.to_owned(), side))
        {
            return;
        }
        let previous = match previous_reference {
            Some(name) => match self.definitions.previous.get(name) {
                Some(definition) => definition,
                None => {
                    return changes.breaks(
                        at,
                        format!("`{name}` is defined nowhere in the previous release"),
                    );
                }
            },
            None => previous,
        };
        let current = match current_reference {
            Some(name) => match self.definitions.current.get(name) {
                Some(definition) => definition,
                None => return changes.breaks(at, format!("`{name}` is defined nowhere")),
            },
            None => current,
        };
        if let (Some(_), Some(_)) = (reference(previous), reference(current)) {
            return self.compare(previous, current, side, at, changes);
        }
        match (previous.is_null(), current.is_null()) {
            (true, true) => return,
            // Data where there was none: what a user sends must now carry
            // it, and what Goodfetch answers says more.
            (true, false) => return changes.narrows(side, at, "added"),
            (false, true) => return changes.breaks(at, "removed"),
            (false, false) => {}
        }
        let empty = Map::new();
        let before = previous.as_object().unwrap_or(&empty);
        let after = current.as_object().unwrap_or(&empty);
        if before.contains_key("anyOf")
            || before.contains_key("oneOf")
            || after.contains_key("anyOf")
            || after.contains_key("oneOf")
        {
            self.compare_variants(previous, current, side, at, changes);
        } else {
            compare_types(before, after, side, at, changes);
            compare_values(before, after, side, at, changes);
            self.compare_properties(before, after, side, at, changes);
            self.compare_additional(before, after, side, at, changes);
            if let (Some(before), Some(after)) = (before.get("items"), after.get("items")) {
                self.compare(before, after, side, &format!("{at}[]"), changes);
            }
        }
        compare_bounds(before, after, side, at, changes);
        for keyword in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            let checked_elsewhere = UNCHECKED_KEYWORDS.contains(&keyword.as_str())
                || COMPARED_KEYWORDS.contains(&keyword.as_str())
                || LOWER_BOUNDS.contains(&keyword.as_str())
                || UPPER_BOUNDS.contains(&keyword.as_str());
            if !checked_elsewhere && before.get(keyword) != after.get(keyword) {
                changes.breaks(at, format!("`{keyword}` changed"));
            }
        }
    }

    fn compare_properties(
        &mut self,
        before: &Map<String, Value>,
        after: &Map<String, Value>,
        side: Side,
        at: &str,
        changes: &mut Changes,
    ) {
        let properties = |schema: &Map<String, Value>| -> Map<String, Value> {
            schema
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        let required = |schema: &Map<String, Value>| -> BTreeSet<String> {
            schema
                .get("required")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let (previous_properties, current_properties) = (properties(before), properties(after));
        let (previous_required, current_required) = (required(before), required(after));
        for (name, previous) in &previous_properties {
            let field = format!("{at}.{name}");
            match current_properties.get(name) {
                None => changes.breaks(&field, "removed"),
                Some(current) => self.compare(previous, current, side, &field, changes),
            }
        }
        for name in current_properties
            .keys()
            .filter(|name| !previous_properties.contains_key(*name))
        {
            let field = format!("{at}.{name}");
            if side != Side::Output && current_required.contains(name) {
                changes.breaks(&field, "added as required");
            } else {
                changes.adds(&field, "added");
            }
        }
        for name in current_required.difference(&previous_required) {
            if previous_properties.contains_key(name) {
                changes.narrows(side, &format!("{at}.{name}"), "now required");
            }
        }
        for name in previous_required.difference(&current_required) {
            if current_properties.contains_key(name) {
                changes.widens(side, &format!("{at}.{name}"), "no longer required");
            }
        }
    }

    fn compare_additional(
        &mut self,
        before: &Map<String, Value>,
        after: &Map<String, Value>,
        side: Side,
        at: &str,
        changes: &mut Changes,
    ) {
        match (
            before.get("additionalProperties"),
            after.get("additionalProperties"),
        ) {
            // What a user sends may accept more, and what Goodfetch answers
            // may say more: the SDKs keep the fields they do not know.
            (Some(Value::Bool(false)), None | Some(Value::Bool(true) | Value::Object(_))) => {
                changes.adds(at, "now takes other properties");
            }
            (None | Some(Value::Bool(true) | Value::Object(_)), Some(Value::Bool(false))) => {
                changes.narrows(side, at, "no longer takes other properties");
            }
            (Some(before @ Value::Object(_)), Some(after @ Value::Object(_))) => {
                self.compare(before, after, side, &format!("{at}{{}}"), changes);
            }
            _ => {}
        }
    }

    /// Each side's `anyOf` or `oneOf` alternatives, matched by their key: a
    /// value they hold, a tag property's value, or the type they name.
    fn compare_variants(
        &mut self,
        previous: &Value,
        current: &Value,
        side: Side,
        at: &str,
        changes: &mut Changes,
    ) {
        let before = variants(previous, self.definitions.previous);
        let after = variants(current, self.definitions.current);
        for (key, previous_variant) in &before {
            match after.get(key) {
                None => changes.narrows(side, at, format!("`{key}` removed")),
                Some(current_variant) => {
                    self.compare(
                        previous_variant,
                        current_variant,
                        side,
                        &format!("{at}<{key}>"),
                        changes,
                    );
                }
            }
        }
        // A new alternative is an addition on both sides: the SDKs keep a
        // value or variant they do not know (ADR 0079).
        for key in after.keys().filter(|key| !before.contains_key(*key)) {
            changes.adds(at, format!("`{key}` added"));
        }
    }
}

/// A schema's `anyOf` or `oneOf` alternatives, or the schema itself, by
/// their key, read with the `$defs` of the file the schema is from.
fn variants(schema: &Value, definitions: &Map<String, Value>) -> BTreeMap<String, Value> {
    let alternatives = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![schema.clone()]);
    alternatives
        .into_iter()
        .map(|alternative| (variant_key(&alternative, definitions), alternative))
        .collect()
}

/// A `anyOf` or `oneOf` alternative's key: the value it holds, else its tag
/// property's value, else the definition it names, else its whole schema
/// without the keywords that validate nothing.
fn variant_key(alternative: &Value, definitions: &Map<String, Value>) -> String {
    if let Some(value) = alternative.get("const") {
        return value.to_string();
    }
    let resolved = reference(alternative)
        .and_then(|name| definitions.get(name))
        .unwrap_or(alternative);
    if let Some(properties) = resolved.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            if let Some(value) = property.get("const") {
                return format!("{name}={value}");
            }
        }
    }
    if let Some(name) = reference(alternative) {
        return name.to_owned();
    }
    without_unchecked_keywords(alternative).to_string()
}

/// `value` without the `UNCHECKED_KEYWORDS`, at every depth.
fn without_unchecked_keywords(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !UNCHECKED_KEYWORDS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), without_unchecked_keywords(value)))
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.iter().map(without_unchecked_keywords).collect())
        }
        other => other.clone(),
    }
}

fn reference(schema: &Value) -> Option<&str> {
    schema.get("$ref")?.as_str()?.strip_prefix("#/$defs/")
}

fn types(schema: &Map<String, Value>) -> Option<BTreeSet<String>> {
    match schema.get("type")? {
        Value::String(name) => Some(BTreeSet::from([name.clone()])),
        Value::Array(names) => Some(
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        _ => None,
    }
}

/// `enum` and `const`, the values a schema allows, as one set.
fn compare_values(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    side: Side,
    at: &str,
    changes: &mut Changes,
) {
    let values = |schema: &Map<String, Value>| -> Option<BTreeSet<String>> {
        if let Some(value) = schema.get("const") {
            return Some(BTreeSet::from([value.to_string()]));
        }
        Some(
            schema
                .get("enum")?
                .as_array()?
                .iter()
                .map(Value::to_string)
                .collect(),
        )
    };
    match (values(before), values(after)) {
        (Some(before), Some(after)) => {
            for removed in before.difference(&after) {
                changes.narrows(side, at, format!("`{}` removed", removed.trim_matches('"')));
            }
            for added in after.difference(&before) {
                changes.adds(at, format!("`{}` added", added.trim_matches('"')));
            }
        }
        (None, Some(_)) => changes.narrows(side, at, "now holds listed values only"),
        (Some(_), None) => changes.widens(side, at, "no longer holds listed values only"),
        (None, None) => {}
    }
}

fn compare_types(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    side: Side,
    at: &str,
    changes: &mut Changes,
) {
    match (types(before), types(after)) {
        (Some(before), Some(after)) => {
            for removed in before.difference(&after) {
                changes.narrows(side, at, format!("no longer of type `{removed}`"));
            }
            for added in after.difference(&before) {
                changes.widens(side, at, format!("now also of type `{added}`"));
            }
        }
        (None, Some(after)) => changes.narrows(side, at, format!("now only of type {after:?}")),
        (Some(before), None) => {
            changes.widens(side, at, format!("no longer only of type {before:?}"));
        }
        (None, None) => {}
    }
}

fn compare_bounds(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    side: Side,
    at: &str,
    changes: &mut Changes,
) {
    for (keywords, lower) in [(LOWER_BOUNDS, true), (UPPER_BOUNDS, false)] {
        for keyword in keywords {
            let bound = |schema: &Map<String, Value>| schema.get(keyword).and_then(Value::as_f64);
            let (previous, current) = (bound(before), bound(after));
            // A bound that appears narrows, and one that goes widens.
            let narrowed = match (previous, current) {
                (Some(previous), Some(current)) if lower => current > previous,
                (Some(previous), Some(current)) => current < previous,
                (previous, current) => previous.is_none() && current.is_some(),
            };
            let widened = match (previous, current) {
                (Some(previous), Some(current)) if lower => current < previous,
                (Some(previous), Some(current)) => current > previous,
                (previous, current) => previous.is_some() && current.is_none(),
            };
            if narrowed {
                changes.narrows(side, at, format!("`{keyword}` narrowed"));
            } else if widened {
                changes.widens(side, at, format!("`{keyword}` widened"));
            }
        }
    }
}
