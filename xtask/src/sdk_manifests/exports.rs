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

use super::{Language, MANIFESTS, Manifest, Member, Operation};
use crate::check::Finding;
use crate::codegen;

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
    /// Its doc text: TypeScript's doc comment without its tags, Rust's `///`
    /// lines, Python's docstring.
    #[serde(default)]
    doc: String,
    /// TypeScript's tag names, such as `experimental`, and `deprecated` for
    /// Rust's `#[deprecated]` or Python's `@deprecated`.
    #[serde(default)]
    markers: Vec<String>,
}

/// The marker of a member serving a deprecated entry, in every language,
/// and TypeScript's of one serving an Experimental entry.
const DEPRECATED: &str = "deprecated";
const EXPERIMENTAL_TAG: &str = "experimental";
/// The catalog's stability of an Experimental entry.
const EXPERIMENTAL: &str = "experimental";
/// The languages, in the order their findings come.
const LANGUAGES: [Language; 3] = [Language::TypeScript, Language::Rust, Language::Python];

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let catalog = super::catalog(root)?;
    let operations: BTreeMap<&str, &Operation> = catalog
        .operations
        .iter()
        .map(|operation| (operation.name.as_str(), operation))
        .collect();
    let generated = codegen::handle_entries(root)?;
    let mut findings = Vec::new();
    let mut manifests = Vec::new();
    for (path, language) in MANIFESTS {
        let exported = exports(root, path, language)?;
        let manifest = super::read(root, path)?;
        compare(path, language, &manifest.members, &exported, &mut findings);
        carried(
            path,
            language,
            &manifest.members,
            &exported,
            &operations,
            &mut findings,
        );
        manifests.push((path, language, manifest));
    }
    for language in LANGUAGES {
        let of_language: Vec<(&str, &Manifest)> = manifests
            .iter()
            .filter(|(_, of, _)| *of == language)
            .map(|(path, _, manifest)| (*path, manifest))
            .collect();
        reached(
            language,
            &of_language,
            &operations,
            &generated,
            &mut findings,
        );
    }
    Ok(findings)
}

/// `text` with each run of whitespace as one space, so a doc wraps its
/// lines as it likes.
fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fails each line of `manifest` whose member lacks its entry's doc text,
/// or the marker of its Experimental or deprecated entry (ADR 0099): its doc
/// holds the entry's description, however it wraps it; an Experimental
/// entry's member carries TypeScript's tag `@experimental`, or the
/// Experimental line of a Rust or Python doc; a deprecated entry's carries
/// TypeScript's tag `@deprecated`, Rust's `#[deprecated]` or Python's
/// `@deprecated`.
fn carried(
    manifest: &str,
    language: Language,
    members: &[Member],
    exported: &[Exported],
    operations: &BTreeMap<&str, &Operation>,
    findings: &mut Vec<Finding>,
) {
    for member in members {
        let Some(operation) = operations.get(member.entry.as_str()) else {
            continue;
        };
        let Some(found) = exported.iter().find(|found| {
            !found.generated
                && !found.library
                && found.entry == member.package
                && found.receiver == member.receiver
                && found.member == member.name
        }) else {
            continue;
        };
        let subject = super::subject(manifest, member);
        let doc = flattened(&found.doc);
        let description = flattened(&operation.description);
        if !doc.contains(&description) {
            findings.push(Finding::new(
                "missing-doc",
                subject.clone(),
                format!(
                    "its doc lacks `{}`'s description; copy it in: \"{description}\" (ADR 0099)",
                    member.entry
                ),
            ));
        }
        let experimental = match language {
            Language::TypeScript => found.markers.iter().any(|tag| tag == EXPERIMENTAL_TAG),
            Language::Rust | Language::Python => doc.contains(codegen::EXPERIMENTAL),
        };
        if operation.stability.as_deref() == Some(EXPERIMENTAL) && !experimental {
            let marker = match language {
                Language::TypeScript => "the TSDoc tag `@experimental`".to_owned(),
                Language::Rust | Language::Python => {
                    format!("the line \"{}\"", codegen::EXPERIMENTAL)
                }
            };
            findings.push(Finding::new(
                "missing-marker",
                subject.clone(),
                format!(
                    "`{}` is Experimental; its doc carries {marker} (ADRs 0030, 0099)",
                    member.entry
                ),
            ));
        }
        if operation.deprecated.is_some() && !found.markers.iter().any(|tag| tag == DEPRECATED) {
            let marker = match language {
                Language::TypeScript => "the TSDoc tag `@deprecated`",
                Language::Rust => "`#[deprecated]`",
                Language::Python => "the `@deprecated` decorator",
            };
            findings.push(Finding::new(
                "missing-marker",
                subject,
                format!(
                    "`{}` is deprecated; the member carries {marker} (ADRs 0047, 0099)",
                    member.entry
                ),
            ));
        }
    }
}

/// Fails each entry of the catalog that no public member of `language`'s
/// SDK serves and no `[[unreached]]` line of its `manifests` names, and each
/// such line naming an entry a member serves, one only a person calls, or
/// none of the catalog's. A member the generator writes on a handle serves
/// its entry; the generated operations table, which no SDK exports, and the
/// raw `call` serve none.
fn reached(
    language: Language,
    manifests: &[(&str, &Manifest)],
    operations: &BTreeMap<&str, &Operation>,
    generated: &BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Some((first, _)) = manifests.first() else {
        return;
    };
    let served: BTreeSet<&str> = manifests
        .iter()
        .flat_map(|(_, manifest)| manifest.members.iter().map(|member| member.entry.as_str()))
        .chain(generated.iter().map(String::as_str))
        .collect();
    let mut unreached = BTreeSet::new();
    for (path, manifest) in manifests {
        for entry in manifest.unreached.iter().flat_map(|line| &line.entries) {
            unreached.insert(entry.as_str());
            let (kind, message) = match operations.get(entry.as_str()) {
                None => ("unknown-entry", "is no operation of the catalog"),
                Some(operation) if operation.person_only() => (
                    "person-only",
                    "only a person calls it, so no SDK reaches it (ADR 0041)",
                ),
                Some(_) if served.contains(entry.as_str()) => (
                    "reached-unreached",
                    "a member of the SDK serves it; remove it from the line",
                ),
                Some(_) => continue,
            };
            findings.push(Finding::new(
                kind,
                format!("{path} {entry}"),
                format!("`{entry}` {message} (ADR 0099)"),
            ));
        }
    }
    let paths: Vec<&str> = manifests.iter().map(|(path, _)| *path).collect();
    for (name, operation) in operations {
        if operation.person_only() || served.contains(name) || unreached.contains(name) {
            continue;
        }
        findings.push(Finding::new(
            "unreached",
            format!("{first} {name}"),
            format!(
                "no public member of the {} SDK serves `{name}`; add one, or name it in an `[[unreached]]` line with its reason in {} (ADR 0099)",
                language.name(),
                paths.join(" or ")
            ),
        ));
    }
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
        let (kind, message) = match by_key.get(&(&member.package, &member.receiver, &member.name)) {
            None => (
                "unused-line",
                format!(
                    "`{}` exports no `{}.{}`; remove the line, or correct its package, receiver or member (ADR 0099)",
                    member.package, member.receiver, member.name
                ),
            ),
            Some(found) if found.iter().all(|found| found.generated) => (
                "generated",
                format!(
                    "the generator writes `{}.{}` of `{}`; remove the line (ADR 0099)",
                    member.receiver, member.name, member.package
                ),
            ),
            Some(_) => continue,
        };
        findings.push(Finding::new(kind, subject, message));
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
