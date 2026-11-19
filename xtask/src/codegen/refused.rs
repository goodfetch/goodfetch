//! The names `xtask codegen` refuses (ADRs 0079 and 0099), so that the
//! catalog renames the entry for every surface and no SDK needs an alias or
//! an override: a name Rust cannot make a raw identifier, a member a
//! language gives meaning on every value, `call`, which is every handle's
//! raw call, an error class that would shadow a built-in one, and two
//! entries that would be one member or one class.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use super::naming::{camel, pascal, rust_snake, rust_type};

/// The members no entry may give a handle, each with why.
const MEMBERS: [(&str, &str); 7] = [
    (
        "then",
        "a value with a `then` member is thenable in TypeScript, so `await` would call it",
    ),
    (
        "constructor",
        "every TypeScript object already has a `constructor`",
    ),
    ("call", "`call` is the raw call every handle has"),
    (
        "clone",
        "every Rust handle implements `Clone`, whose method is `clone`",
    ),
    (
        "eq",
        "every Rust handle implements `PartialEq`, whose method is `eq`",
    ),
    (
        "fmt",
        "every Rust handle implements `Debug`, whose method is `fmt`",
    ),
    (
        "hash",
        "every Rust handle implements `Hash`, whose method is `hash`",
    ),
];

/// The error classes TypeScript, the web platform and Python define, which
/// a generated class of the same name would shadow (ADR 0079).
const BUILT_IN_ERRORS: [&str; 56] = [
    "AbortError",
    "AggregateError",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "EOFError",
    "EnvironmentError",
    "Error",
    "EvalError",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "IOError",
    "ImportError",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RangeError",
    "RecursionError",
    "ReferenceError",
    "RuntimeError",
    "SuppressedError",
    "SyntaxError",
    "SystemError",
    "TabError",
    "TimeoutError",
    "TypeError",
    "URIError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "ValueError",
    "ZeroDivisionError",
];

/// What the SDKs declare beside the error classes under an `Error` name:
/// the base class, and each Product's union of its errors (ADR 0079).
const SDK_ERRORS: [&str; 3] = ["CollieError", "FoxhoundError", "GoodfetchError"];

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct ErrorCode {
    name: String,
    group: String,
}

#[derive(Deserialize)]
struct PluginKind {
    name: String,
    methods: Vec<Named>,
}

/// The sections of a projection whose names become members and classes.
#[derive(Deserialize)]
struct Names {
    operations: Vec<Named>,
    events: Vec<Named>,
    error_groups: Vec<Named>,
    error_codes: Vec<ErrorCode>,
    plugin_kinds: Vec<PluginKind>,
}

/// Fails, naming each entry, when a Product's catalog holds a name no SDK
/// can spell by its one rule. Each of `projections` is one Product's whole
/// catalog, so two entries of different Products that one Product carries
/// are read together.
pub(super) fn check(projections: &[&Value]) -> Result<()> {
    let mut refused = BTreeSet::new();
    for projection in projections {
        let names = Names::deserialize(*projection).context("reading a projection's names")?;
        refused.extend(refusals(&names));
    }
    if refused.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = refused
        .into_iter()
        .map(|line| format!("- {line}"))
        .collect();
    bail!(
        "the catalog holds names no SDK can spell by its one rule (ADRs 0079 and 0099); rename each entry, for every surface:\n{}",
        list.join("\n")
    );
}

/// `entries`, each in backquotes, joined by "and".
fn listed(entries: &[String]) -> String {
    let entries: Vec<String> = entries.iter().map(|entry| format!("`{entry}`")).collect();
    entries.join(" and ")
}

/// Whether `part` is a Python dunder name, such as `__init__`.
fn is_dunder(part: &str) -> bool {
    part.len() > 4 && part.starts_with("__") && part.ends_with("__")
}

/// Why each refused entry of `names` is refused, one line an entry.
fn refusals(names: &Names) -> BTreeSet<String> {
    let mut refused = BTreeSet::new();
    let operations = names
        .operations
        .iter()
        .map(|operation| ("operation", operation.name.clone()));
    let methods = names.plugin_kinds.iter().flat_map(|kind| {
        kind.methods.iter().map(|method| {
            (
                "Plugin kind method",
                format!("{}.{}", kind.name, method.name),
            )
        })
    });
    // Each member by its whole name's spelling, which is the same wherever
    // a dot parts its words, with the entries that would be it.
    let mut members: BTreeMap<(&str, String), Vec<String>> = BTreeMap::new();
    for (what, entry) in operations.chain(methods) {
        let member = entry
            .rsplit_once('.')
            .map_or(entry.as_str(), |(_, member)| member);
        if let Some((_, why)) = MEMBERS.iter().find(|(name, _)| *name == camel(member)) {
            refused.insert(format!("{what} `{entry}`: {why}"));
        }
        for part in entry.split('.') {
            if rust_snake(part).is_none() {
                refused.insert(format!(
                    "{what} `{entry}`: Rust cannot make `{part}` a raw identifier"
                ));
            }
            if is_dunder(part) {
                refused.insert(format!(
                    "{what} `{entry}`: `{part}` is a Python dunder name, which Python gives meaning"
                ));
            }
        }
        members
            .entry((what, camel(&entry)))
            .or_default()
            .push(entry);
    }
    // Each Rust type an entry names: a Plugin kind's trait, and an event's
    // marker, from the last part of its name.
    let kinds = names
        .plugin_kinds
        .iter()
        .map(|kind| ("Plugin kind", &kind.name, kind.name.as_str()));
    let events = names.events.iter().map(|event| {
        let marker = event
            .name
            .rsplit_once('.')
            .map_or(event.name.as_str(), |(_, marker)| marker);
        ("event", &event.name, marker)
    });
    for (what, entry, words) in kinds.chain(events) {
        if rust_type(words).is_none() {
            refused.insert(format!(
                "{what} `{entry}`: its Rust type would be `Self`, which Rust cannot make a raw identifier"
            ));
        }
    }
    for ((_, member), entries) in &members {
        if entries.len() > 1 {
            refused.insert(format!("{} are one member, `{member}`", listed(entries)));
        }
    }
    // Each error class with the entries that would be it. A code named as
    // its own group is that group's class, which the generators write once.
    let mut classes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let groups = names
        .error_groups
        .iter()
        .map(|group| ("error group", &group.name));
    let codes = names
        .error_codes
        .iter()
        .filter(|code| code.name != code.group)
        .map(|code| ("Error code", &code.name));
    for (what, name) in groups.chain(codes) {
        let class = format!("{}Error", pascal(name));
        if BUILT_IN_ERRORS.contains(&class.as_str()) {
            refused.insert(format!(
                "{what} `{name}`: its class `{class}` would shadow the built-in `{class}`"
            ));
        }
        if SDK_ERRORS.contains(&class.as_str()) {
            refused.insert(format!(
                "{what} `{name}`: its class `{class}` would shadow the SDK's own `{class}`"
            ));
        }
        classes
            .entry(class)
            .or_default()
            .push(format!("{what} `{name}`"));
    }
    for (class, entries) in &classes {
        if entries.len() > 1 {
            refused.insert(format!(
                "{} are one class, `{class}`",
                entries.join(" and ")
            ));
        }
    }
    refused
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde::Deserialize as _;
    use serde_json::{Value, json};

    use super::{Names, refusals};

    fn refused(operations: &[&str], error_codes: &Value) -> Vec<String> {
        let operations: Vec<Value> = operations
            .iter()
            .map(|name| json!({ "name": name }))
            .collect();
        let names = Names::deserialize(json!({
            "operations": operations,
            "events": [],
            "error_groups": [{ "name": "refused" }, { "name": "internal" }],
            "error_codes": error_codes,
            "plugin_kinds": [],
        }))
        .unwrap();
        refusals(&names).into_iter().collect()
    }

    /// The TypeScript layer spells a member from its whole name, so a dot
    /// that moves between the same words gives the same member.
    ///
    /// Covers: story:11-176
    #[rstest]
    #[case::one_namespace(
        &["page.to_http_s", "page.to_https"],
        "`page.to_http_s` and `page.to_https` are one member, `pageToHTTPS`"
    )]
    #[case::a_dot_between_the_same_words(
        &["sample.to_x", "sample_to.x"],
        "`sample.to_x` and `sample_to.x` are one member, `sampleToX`"
    )]
    fn two_operations_of_one_spelling_are_refused(#[case] operations: &[&str], #[case] line: &str) {
        assert_eq!(refused(operations, &json!([])), vec![line.to_owned()]);
    }

    /// Covers: story:11-176
    #[rstest]
    #[case::two_codes(
        json!([{ "name": "http_s", "group": "refused" }, { "name": "https", "group": "refused" }]),
        vec!["Error code `http_s` and Error code `https` are one class, `HTTPSError`".to_owned()]
    )]
    #[case::a_code_named_as_another_group(
        json!([{ "name": "internal", "group": "refused" }]),
        vec!["error group `internal` and Error code `internal` are one class, `InternalError`".to_owned()]
    )]
    #[case::a_code_named_as_its_own_group(json!([{ "name": "internal", "group": "internal" }]), vec![])]
    fn two_errors_of_one_class_are_refused(#[case] error_codes: Value, #[case] lines: Vec<String>) {
        assert_eq!(refused(&[], &error_codes), lines);
    }

    /// A code's class may not take a built-in's name, nor the name of what
    /// the SDKs declare beside the classes: the base class and each
    /// Product's union of its errors.
    ///
    /// Covers: story:11-176
    #[rstest]
    #[case::a_built_in(
        "type",
        "Error code `type`: its class `TypeError` would shadow the built-in `TypeError`"
    )]
    #[case::the_base_class(
        "goodfetch",
        "Error code `goodfetch`: its class `GoodfetchError` would shadow the SDK's own `GoodfetchError`"
    )]
    #[case::a_products_union(
        "foxhound",
        "Error code `foxhound`: its class `FoxhoundError` would shadow the SDK's own `FoxhoundError`"
    )]
    fn a_code_whose_class_is_taken_is_refused(#[case] code: &str, #[case] line: &str) {
        assert_eq!(
            refused(&[], &json!([{ "name": code, "group": "refused" }])),
            vec![line.to_owned()]
        );
    }

    /// Covers: story:11-176
    #[test]
    fn a_group_whose_class_is_a_built_in_one_is_refused() {
        let names = Names::deserialize(json!({
            "operations": [],
            "events": [],
            "error_groups": [{ "name": "timeout" }],
            "error_codes": [],
            "plugin_kinds": [],
        }))
        .unwrap();

        assert_eq!(
            refusals(&names).into_iter().collect::<Vec<_>>(),
            vec![
                "error group `timeout`: its class `TimeoutError` would shadow the built-in `TimeoutError`"
                    .to_owned()
            ]
        );
    }
}
