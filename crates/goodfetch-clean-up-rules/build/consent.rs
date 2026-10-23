//! The Consent prompt runner's bundle (ADR 0048): one script the engine
//! evaluates in the guard's isolated world of every frame. It holds
//! autoconsent's library and heuristics, each evaluated as its authors
//! published it, autoconsent's rules less every rule that names a dropped
//! snippet, Goodfetch's snippet table, and Goodfetch's runner, which it
//! calls with all four. The build refuses a snippet the library has that
//! the table does not class, and one the table classes that the library no
//! longer has. The bundle is embedded zstd-compressed (ADR 0057).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

const LIBRARY: &str = "consent/autoconsent/autoconsent.cjs.js";
const HEURISTICS: &str = "consent/autoconsent/heuristics.cjs.js";
const RULES: &str = "consent/autoconsent/compact-rules.json";
const SNIPPETS: &str = "consent/snippets.toml";
const RUNNER: &str = "consent/runner.js";

/// The positions, in a compact rule, of its step lists: `detectCmp`,
/// `detectPopup`, `optOut` and `test`; and of its name.
const STEP_LISTS: [usize; 4] = [6, 7, 8, 9];
const NAME: usize = 1;

/// zstd's level for the bundle.
const LEVEL: i32 = 19;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    snippets: BTreeMap<String, World>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum World {
    Isolated,
    Main,
    Dropped,
}

/// Writes the bundle and its Rust tables to `out`.
pub(crate) fn bundle(manifest: &Path, out: &Path) {
    let read = |path: &str| {
        std::fs::read_to_string(manifest.join(path))
            .unwrap_or_else(|error| panic!("invariant: the crate carries {path}: {error}"))
    };
    let library = read(LIBRARY);
    let table: Table = toml::from_str(&read(SNIPPETS))
        .unwrap_or_else(|error| panic!("invariant: {SNIPPETS} is well formed: {error}"));
    let known = library_snippets(&library);
    let classed: BTreeSet<&str> = table.snippets.keys().map(String::as_str).collect();
    let unclassed: Vec<&&str> = known.difference(&classed).collect();
    let gone: Vec<&&str> = classed.difference(&known).collect();
    assert!(
        unclassed.is_empty() && gone.is_empty(),
        "{SNIPPETS} classes no world for {unclassed:?}, and classes {gone:?}, which {LIBRARY} no longer has"
    );
    let dropped: BTreeSet<&str> = table
        .snippets
        .iter()
        .filter(|(_, world)| **world == World::Dropped)
        .map(|(snippet, _)| snippet.as_str())
        .collect();

    let mut rules: Value = serde_json::from_str(&read(RULES))
        .unwrap_or_else(|error| panic!("invariant: {RULES} is JSON: {error}"));
    // The index ranges name positions in the full list, which no longer
    // hold once rules are dropped; the library reads rules without them.
    if let Some(object) = rules.as_object_mut() {
        object.remove("index");
    }
    let mut left_out = Vec::new();
    if let Some(list) = rules.get_mut("r").and_then(Value::as_array_mut) {
        list.retain(|rule| {
            let names = STEP_LISTS
                .iter()
                .filter_map(|position| rule.get(position))
                .any(|steps| names_any(steps, &dropped));
            if names {
                left_out.push(rule[NAME].as_str().unwrap_or_default().to_owned());
            }
            !names
        });
    }
    let worlds: BTreeMap<&str, &str> = table
        .snippets
        .iter()
        .map(|(snippet, world)| {
            (
                snippet.as_str(),
                match world {
                    World::Isolated => "isolated",
                    World::Main => "main",
                    World::Dropped => "dropped",
                },
            )
        })
        .collect();
    let module = |path: &str| {
        format!(
            "(() => {{\nconst module = {{ exports: {{}} }};\nconst exports = module.exports;\n{}\nreturn module.exports;\n}})()",
            read(path)
        )
    };
    let source = format!(
        "(() => {{\nconst autoconsent = {};\nconst heuristics = {};\nconst rules = {};\nconst worlds = {};\n{}\nrunner({{ autoconsent, heuristics, rules, worlds }});\n}})();\n",
        module(LIBRARY),
        module(HEURISTICS),
        serde_json::to_string(&rules).expect("invariant: JSON serializes"),
        serde_json::to_string(&worlds).expect("invariant: JSON serializes"),
        read(RUNNER),
    );
    let compressed = zstd::bulk::compress(source.as_bytes(), LEVEL)
        .expect("invariant: zstd compresses a buffer in memory");
    std::fs::write(out.join("consent.js.zst"), &compressed)
        .expect("invariant: a build script can write its OUT_DIR");

    let mut tables = String::from(
        "/// The runner's bundle, compressed, and its size.\n\
         pub(crate) const BUNDLE: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/consent.js.zst\"));\n",
    );
    writeln!(
        tables,
        "pub(crate) const BUNDLE_SIZE: usize = {};",
        super::separated(source.len())
    )
    .expect("invariant: writing to a String cannot fail");
    tables.push_str("pub(crate) const SNIPPETS: &[(&str, World)] = &[\n");
    for (snippet, world) in &table.snippets {
        writeln!(tables, "    ({snippet:?}, World::{world:?}),")
            .expect("invariant: writing to a String cannot fail");
    }
    tables.push_str("];\n");
    writeln!(
        tables,
        "pub(crate) const DROPPED_RULES: &[&str] = &{left_out:?};"
    )
    .expect("invariant: writing to a String cannot fail");
    std::fs::write(out.join("consent.rs"), tables)
        .expect("invariant: a build script can write its OUT_DIR");
}

/// The snippets the library's `snippets` object holds, by name.
fn library_snippets(library: &str) -> BTreeSet<&str> {
    let start = library
        .find("\nvar snippets = {\n")
        .unwrap_or_else(|| panic!("invariant: {LIBRARY} holds its snippets object"));
    let body = &library[start..];
    let end = body
        .find("\n};\n")
        .unwrap_or_else(|| panic!("invariant: {LIBRARY}'s snippets object ends"));
    body[..end]
        .lines()
        .filter_map(|line| {
            let name = line.strip_prefix("  ")?.split(':').next()?;
            (name.starts_with("EVAL_")
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .then_some(name)
        })
        .collect()
}

/// Whether `steps`, a rule's step list or one step, names a snippet of
/// `dropped`, at any depth (`if`, `then`, `else`, `any`).
fn names_any(steps: &Value, dropped: &BTreeSet<&str>) -> bool {
    match steps {
        Value::Array(steps) => steps.iter().any(|step| names_any(step, dropped)),
        Value::Object(step) => step.iter().any(|(key, value)| match key.as_str() {
            "eval" => value
                .as_str()
                .is_some_and(|snippet| dropped.contains(snippet)),
            "if" | "then" | "else" | "any" => names_any(value, dropped),
            _ => false,
        }),
        _ => false,
    }
}
