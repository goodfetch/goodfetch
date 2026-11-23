//! The API breakage check of the TypeScript and Python SDK packages (ADRs
//! 0030, 0069 and 0105, issue 148): each package's public API, testing
//! entry points and modules included, against the commit `compat/api.toml`
//! names as the previous release, whose `sdks` folder and the TypeScript
//! settings its packages extend `git archive` writes into a temporary
//! folder. Both sides are read by the tools the workspace installs, free and
//! run locally: TypeScript and API Extractor read each TypeScript package
//! (`typescript-api.mjs` says how), and Griffe each Python package
//! (`python_api.py` says how). A package a tree lacks has no API there.
//!
//! A TypeScript item, a declaration or one of its members, is keyed by its
//! entry point and its canonical reference, and its subject is
//! `typescript <entry> <name>`, `<name>` the entry's declaration it is or
//! belongs to:
//!
//! - an item gone is a break, and so is a changed signature, except one
//!   whose parameters only grow by optional ones or turn optional, which
//!   adds to it;
//! - a new item is an addition, except a property an interface that was
//!   there now requires, which breaks the code building one;
//! - an Experimental item, one whose doc comment or an enclosing
//!   declaration's has `@experimental`, changes and goes freely (ADR 0030).
//!
//! Each breakage Griffe finds, such as a public object removed or a
//! parameter that became required, is a break of `python <module> <name>`,
//! except one of a function calling `warn_experimental(`, the Python SDK's
//! Experimental member. Griffe reports no additions.
//!
//! The generator's code, under a `generated` folder or module, is left to
//! `xtask api-check`, which judges the catalog it comes from.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::api_baseline::{self, Changes, PYTHON, TYPESCRIPT};

/// Writes the items of the TypeScript packages' entry points.
const TYPESCRIPT_API: &str = include_str!("sdk_api/typescript-api.mjs");
/// Prints the breakages of the Python packages.
const PYTHON_API: &str = include_str!("sdk_api/python_api.py");
/// The repository's interpreter, which `uv sync` installs.
const PYTHON_INTERPRETER: &str = ".venv/bin/python";

/// The TypeScript packages under semver (ADR 0095), in the order a package
/// comes after those it re-exports.
const TYPESCRIPT_PACKAGES: [&str; 3] = [
    "sdks/typescript/collie-client",
    "sdks/typescript/collie",
    "sdks/typescript/foxhound",
];
/// The Python packages under semver: each one's source folder and import
/// name.
const PYTHON_PACKAGES: [(&str, &str); 3] = [
    ("sdks/python/collie-client/src", "goodfetch_collie_client"),
    ("sdks/python/collie/src", "goodfetch_collie"),
    ("sdks/python/foxhound/src", "goodfetch_foxhound"),
];
/// What the previous release's tree needs: the SDKs, and the TypeScript
/// settings their `tsconfig.json` files extend.
const ARCHIVED: [&str; 2] = ["sdks", "tsconfig.base.json"];

/// One declaration or member of a TypeScript entry point.
#[derive(Deserialize)]
struct Item {
    entry: String,
    top: String,
    key: String,
    signature: String,
    /// A function's, method's or constructor's parameters.
    #[serde(default)]
    parameters: Option<Vec<Parameter>>,
    #[serde(default)]
    returns: Option<String>,
    experimental: bool,
    required_member: bool,
}

#[derive(Deserialize)]
struct Parameter {
    #[serde(rename = "type")]
    of_type: String,
    optional: bool,
}

/// One breakage Griffe found in a Python package.
#[derive(Deserialize)]
struct PythonBreak {
    module: String,
    top: String,
    path: String,
    kind: String,
    experimental: bool,
}

pub(crate) fn run(root: &Path) -> Result<ExitCode> {
    let baseline = api_baseline::baseline(root)?;
    let folder = tempfile::tempdir().context("creating a folder for the previous release")?;
    let previous = folder.path().join("previous");
    api_baseline::archive(root, &baseline.commit, &ARCHIVED, &previous)?;
    let mut entries: BTreeMap<String, Changes> = BTreeMap::new();
    let before = typescript_items(root, &previous, &folder.path().join("typescript-previous"))?;
    let after = typescript_items(root, root, &folder.path().join("typescript-current"))?;
    compare_typescript(&before, &after, &mut entries);
    for found in python_breaks(root, &previous)? {
        if !found.experimental {
            entries
                .entry(format!("{PYTHON}{} {}", found.module, found.top))
                .or_default()
                .breaks(&found.path, &found.kind);
        }
    }
    Ok(baseline
        .verdict(
            |entry| entry.starts_with(TYPESCRIPT) || entry.starts_with(PYTHON),
            &entries,
        )
        .report())
}

/// The items of the TypeScript packages of `tree`, written under `out`.
fn typescript_items(root: &Path, tree: &Path, out: &Path) -> Result<Vec<Item>> {
    let request = serde_json::json!({
        "tree": tree,
        "out": out,
        "packages": TYPESCRIPT_PACKAGES,
    });
    let output = Command::new("node")
        .args(["--input-type=module", "--eval", TYPESCRIPT_API])
        .arg(request.to_string())
        .current_dir(root)
        .output()
        .context("running node to read the TypeScript packages' API")?;
    if !output.status.success() {
        bail!(
            "reading the TypeScript packages' API under {} failed ({}): {}",
            tree.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let items = out.join("items.json");
    let text = std::fs::read(&items).with_context(|| format!("reading {}", items.display()))?;
    serde_json::from_slice(&text).with_context(|| format!("parsing {}", items.display()))
}

/// Records each change from `before`'s items to `after`'s.
fn compare_typescript(before: &[Item], after: &[Item], entries: &mut BTreeMap<String, Changes>) {
    let old = keyed(before);
    let new = keyed(after);
    for (at, was) in &old {
        match new.get(at) {
            _ if was.experimental => {}
            None => changes_of(entries, was).breaks(&was.key, "removed"),
            Some(now) if now.signature == was.signature || now.experimental => {}
            Some(now) => {
                let what = format!("`{}` is now `{}`", was.signature, now.signature);
                if widened(was, now) {
                    changes_of(entries, was).adds(&was.key, what);
                } else {
                    changes_of(entries, was).breaks(&was.key, what);
                }
            }
        }
    }
    for (at, now) in &new {
        if old.contains_key(at) {
            continue;
        }
        let changes = changes_of(entries, now);
        let required = now.required_member
            && !now.experimental
            && old
                .values()
                .any(|was| was.entry == now.entry && was.top == now.top);
        if required {
            changes.breaks(&now.key, format!("`{}` is now required", now.signature));
        } else {
            changes.adds(&now.key, format!("`{}`", now.signature));
        }
    }
}

/// The changes of the subject `item` belongs to.
fn changes_of<'a>(entries: &'a mut BTreeMap<String, Changes>, item: &Item) -> &'a mut Changes {
    entries
        .entry(format!("{TYPESCRIPT}{} {}", item.entry, item.top))
        .or_default()
}

/// The items by entry point and key.
fn keyed(items: &[Item]) -> BTreeMap<(&str, &str), &Item> {
    items
        .iter()
        .map(|item| ((item.entry.as_str(), item.key.as_str()), item))
        .collect()
}

/// Whether `now` calls as `was` does and more: the same text before its
/// parameters and the same return type, each parameter of `was` with the
/// same type and optional if it was, and each new one optional.
fn widened(was: &Item, now: &Item) -> bool {
    let head = |item: &Item| item.signature.split('(').next().map(str::to_owned);
    let (Some(before), Some(after)) = (&was.parameters, &now.parameters) else {
        return false;
    };
    head(was) == head(now)
        && was.returns == now.returns
        && after.len() >= before.len()
        && before
            .iter()
            .zip(after)
            .all(|(a, b)| a.of_type == b.of_type && (b.optional || !a.optional))
        && after[before.len()..]
            .iter()
            .all(|parameter| parameter.optional)
}

/// Griffe's breakages of the Python packages of the workspace against those
/// of `previous`, read in an isolated interpreter that imports none of them.
fn python_breaks(root: &Path, previous: &Path) -> Result<Vec<PythonBreak>> {
    let python = root.join(PYTHON_INTERPRETER);
    if !python.is_file() {
        bail!(
            "{} is missing; run `uv sync` to read the Python packages' API",
            python.display()
        );
    }
    let request = serde_json::json!({
        "previous": previous,
        "current": root,
        "packages": PYTHON_PACKAGES,
    });
    let output = Command::new(python)
        .args(["-I", "-c", PYTHON_API])
        .arg(request.to_string())
        .current_dir(root)
        .output()
        .context("running Python to read the Python packages' API")?;
    if !output.status.success() {
        bail!(
            "reading the Python packages' API failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing the Python packages' breakages")
}
