//! `xtask sdk-exports`, in the Quick suite: each SDK's manifest lists every
//! public hand-written member its packages export, and no member they do not
//! export or the generator writes (ADRs 0069, 0099). A member is listed when
//! a line names it on any entry that exports it, so a member that several
//! entries re-export, or a subclass inherits, is listed once. TypeScript's
//! members are read by TypeScript's checker (`typescript-exports.mjs` says
//! which), a Rust crate's from its source ([`rust`] says which), and the
//! Python packages' from their source by the repository's interpreter
//! (`python_exports.py` says which).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Language, MANIFESTS, Member};
use crate::check::Finding;

mod rust;

/// Prints the members the TypeScript packages of the SDK's folder export.
const TYPESCRIPT_EXPORTS: &str = include_str!("typescript-exports.mjs");
/// Prints the members the Python packages of the SDK's folder export.
const PYTHON_EXPORTS: &str = include_str!("python_exports.py");
/// The repository's interpreter, which `uv sync` installs.
const PYTHON: &str = ".venv/bin/python";

/// One member an SDK's entry exports.
#[derive(Deserialize)]
struct Exported {
    /// The entry exporting it: an npm package and its subpath, a crate, or
    /// a Python module.
    entry: String,
    receiver: String,
    member: String,
    /// The same for every entry exporting one declaration.
    id: String,
    /// The package whose entry exports it: a TypeScript package's folder,
    /// a crate's name, or a Python package's import name.
    home: String,
    /// The package declaring it, named as `home` is, `None` for the
    /// language library's.
    package: Option<String>,
    generated: bool,
    /// Declared by the language's library, such as `Disposable`'s
    /// `[Symbol.dispose]`: a line may name it and none has to.
    library: bool,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for (manifest, language) in MANIFESTS {
        let exported = exports(root, manifest, language)?;
        compare(
            manifest,
            language,
            &super::read(root, manifest)?,
            &exported,
            &mut findings,
        );
    }
    Ok(findings)
}

/// The members the SDK of `manifest` exports.
fn exports(root: &Path, manifest: &str, language: Language) -> Result<Vec<Exported>> {
    let sdk = Path::new(manifest)
        .parent()
        .context("a manifest sits in its SDK's folder")?;
    match language {
        Language::TypeScript => typescript_exports(root, sdk),
        Language::Rust => {
            let name = sdk
                .file_name()
                .and_then(|name| name.to_str())
                .context("a Rust manifest sits in its crate's folder, named as the crate")?;
            rust::exports(&root.join(sdk), name)
        }
        Language::Python => python_exports(root, sdk),
    }
}

/// The members the Python packages of the folder `sdk` export, read in an
/// isolated interpreter that imports none of them.
fn python_exports(root: &Path, sdk: &Path) -> Result<Vec<Exported>> {
    let python = root.join(PYTHON);
    if !python.is_file() {
        bail!(
            "{} is missing; run `uv sync` to read the Python SDK's exports",
            python.display()
        );
    }
    let output = Command::new(python)
        .args(["-I", "-c", PYTHON_EXPORTS])
        .arg(root.join(sdk))
        .current_dir(root)
        .output()
        .context("running Python to read the Python SDK's exports")?;
    if !output.status.success() {
        bail!(
            "reading the Python SDK's exports failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing the Python SDK's exports")
}

/// The members the TypeScript packages of the folder `sdk` export.
fn typescript_exports(root: &Path, sdk: &Path) -> Result<Vec<Exported>> {
    let output = Command::new("node")
        .args(["--input-type=module", "--eval", TYPESCRIPT_EXPORTS])
        .arg(root.join(sdk))
        .current_dir(root)
        .output()
        .context("running node to read the TypeScript SDK's exports")?;
    if !output.status.success() {
        bail!(
            "reading the TypeScript SDK's exports failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing the TypeScript SDK's exports")
}

/// Fails each line of `manifest` naming no hand-written member its SDK
/// exports, and each hand-written member no line lists.
fn compare(
    manifest: &str,
    language: Language,
    members: &[Member],
    exported: &[Exported],
    findings: &mut Vec<Finding>,
) {
    // The key a line names its package with.
    let package = match language {
        Language::TypeScript | Language::Python => "package",
        Language::Rust => "crate",
    };
    let mut by_key: BTreeMap<(&str, &str, &str), Vec<&Exported>> = BTreeMap::new();
    for member in exported {
        by_key
            .entry((&member.entry, &member.receiver, &member.member))
            .or_default()
            .push(member);
    }
    let listed: BTreeSet<(&str, &str, &str)> = members
        .iter()
        .map(|member| {
            (
                member.package.as_str(),
                member.receiver.as_str(),
                member.name.as_str(),
            )
        })
        .collect();
    for member in members {
        let subject = super::subject(manifest, member);
        let message = match by_key.get(&(&member.package, &member.receiver, &member.name)) {
            None => format!(
                "`{}` exports no `{}.{}`; remove the line, or correct its package, receiver or member (ADR 0099)",
                member.package, member.receiver, member.name
            ),
            Some(found) if found.iter().all(|found| found.generated) => format!(
                "the generator writes `{}.{}` of `{}`; remove the line (ADR 0099)",
                member.receiver, member.name, member.package
            ),
            Some(_) => continue,
        };
        findings.push(Finding::new("unused-line", subject, message));
    }
    // Each entry's receivers, and the members it lists on `(module)`.
    let names: BTreeSet<(&str, &str)> = members
        .iter()
        .flat_map(|member| {
            [
                (member.package.as_str(), member.receiver.as_str()),
                (member.package.as_str(), member.name.as_str()),
            ]
        })
        .collect();
    let mut declarations: BTreeMap<&str, Vec<&Exported>> = BTreeMap::new();
    let mut order = Vec::new();
    for member in exported
        .iter()
        .filter(|member| !member.generated && !member.library)
    {
        let held = declarations.entry(&member.id).or_default();
        if held.is_empty() {
            order.push(member.id.as_str());
        }
        held.push(member);
    }
    for id in order {
        let held = &declarations[id];
        if held.iter().any(|member| {
            listed.contains(&(
                member.entry.as_str(),
                member.receiver.as_str(),
                member.member.as_str(),
            ))
        }) {
            continue;
        }
        // An entry whose lines already name the receiver, else the
        // declaring package's own entry, the shortest first.
        let shown = held
            .iter()
            .min_by_key(|member| {
                (
                    !names.contains(&(member.entry.as_str(), member.receiver.as_str())),
                    member.package.as_deref() != Some(member.home.as_str()),
                    member.entry.len(),
                )
            })
            .expect("invariant: each declaration groups at least one export");
        findings.push(Finding::new(
            "unlisted",
            format!("{manifest} {}.{}", shown.receiver, shown.member),
            format!(
                "`{}` exports it and no line lists it; add a `[[member]]` line with {package} = \"{}\", receiver = \"{}\", member = \"{}\", the entry it serves or entry = \"none\" with its reason, its words and its form (ADR 0099)",
                shown.entry, shown.entry, shown.receiver, shown.member
            ),
        ));
    }
}
