//! `xtask sdk-manifests`, in the Quick suite: every member an SDK's manifest
//! of hand-written public members lists serves a catalog operation or none
//! (ADR 0099), and never one only a person may call (`reach =
//! "person_only"`, ADR 0041), such as `usage.raise`: no SDK member raises a
//! Limit (ADR 0080). It also fails, as ADR 0099's "Generated and
//! hand-written" lists:
//!
//! - two members of one receiver listed for the same entry;
//! - a member with no entry not spelled from the words and form its line
//!   declares, by its language's rule for a catalog name;
//! - a line naming a member the generator writes on that receiver for that
//!   entry, which is generated instead;
//! - a member whose words are missing from the same receiver in another
//!   SDK's manifest, unless its line's `only` says why that SDK lacks it.
//!
//! A receiver is the same across SDKs within the same package, the
//! `@goodfetch/` or `goodfetch-` prefix aside; a Python package is its import
//! name, `goodfetch_collie_client` as `collie-client` and a submodule such as
//! `goodfetch_collie.testing` as the subpath `collie/testing`. A Rust
//! `blocking::` twin is its async receiver, a Python `Async<Name>` twin is
//! `<Name>` (ADR 0081), and a Rust crate's root is a TypeScript module's.
//! `xtask sdk-exports` matches a manifest to the members its SDK exports
//! ([`exports`]), TypeScript's, Rust's and Python's, checks that each
//! listed member carries its entry's doc text and markers, and that each
//! entry reaches a public member of every SDK or an `[[unreached]]` line
//! says why it does not.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::codegen;

pub(crate) mod exports;

/// The catalog `xtask codegen` emits from `goodfetch-protocol`'s types.
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";
/// Each SDK's manifest and its language; an SDK that lists its hand-written
/// members adds its own here.
const MANIFESTS: [(&str, Language); 5] = [
    ("sdks/typescript/manifest.toml", Language::TypeScript),
    (
        "crates/goodfetch-collie-client/manifest.toml",
        Language::Rust,
    ),
    ("crates/goodfetch-collie/manifest.toml", Language::Rust),
    (
        "crates/foxhound/goodfetch-foxhound/manifest.toml",
        Language::Rust,
    ),
    ("sdks/python/manifest.toml", Language::Python),
];
/// The `entry` of a member that serves no catalog entry, with its reason.
const NO_ENTRY: &str = "none";
/// The reach of an operation only a person's surface offers.
const PERSON_ONLY: &str = "person_only";
/// The forms a manifest line declares, as `sdks/typescript/manifest.toml`'s
/// header describes them.
const FORMS: [&str; 7] = [
    "opener",
    "round_trip",
    "read",
    "closer",
    "runtime",
    "type",
    "constant",
];
/// The form spelled as a type or class name.
const TYPE: &str = "type";
/// The form spelled in capitals, its words joined by `_`.
const CONSTANT: &str = "constant";
/// The prefix of a Rust receiver's blocking twin (ADR 0081).
const BLOCKING: &str = "blocking::";
/// The prefix of a Python receiver's async twin (ADR 0081).
const ASYNC: &str = "Async";
/// The prefix of a Python package's import name.
const PYTHON_PREFIX: &str = "goodfetch_";
/// The receiver of a member at a TypeScript module's top level; a Rust
/// crate's root is `(crate)`.
const MODULE: &str = "(module)";
const CRATE: &str = "(crate)";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    TypeScript,
    Rust,
    Python,
}

impl Language {
    fn name(self) -> &'static str {
        match self {
            Self::TypeScript => "TypeScript",
            Self::Rust => "Rust",
            Self::Python => "Python",
        }
    }
}

#[derive(Deserialize)]
struct Catalog {
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
struct Operation {
    name: String,
    #[serde(default)]
    reach: Option<String>,
    /// Its doc text, which each member serving it carries.
    #[serde(default)]
    description: String,
    /// `experimental` or `stable`.
    #[serde(default)]
    stability: Option<String>,
    /// Its deprecation, `null` when it is not deprecated.
    #[serde(default)]
    deprecated: Option<serde_json::Value>,
}

impl Operation {
    fn person_only(&self) -> bool {
        self.reach.as_deref() == Some(PERSON_ONLY)
    }
}

/// The catalog, read from `root`.
fn catalog(root: &Path) -> Result<Catalog> {
    let path = root.join(CATALOG);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "member", default)]
    members: Vec<Member>,
    /// The entries no member of the SDK's language serves yet.
    #[serde(default)]
    unreached: Vec<Unreached>,
}

/// An `[[unreached]]` line: entries no public member of the language's SDK
/// serves, and why.
#[derive(Deserialize)]
struct Unreached {
    entries: Vec<String>,
    #[expect(
        dead_code,
        reason = "required, so a line without its reason fails to parse; no check reads it"
    )]
    reason: String,
}

#[derive(Deserialize)]
struct Member {
    /// The npm package, or the crate as `crate`.
    #[serde(alias = "crate")]
    package: String,
    receiver: String,
    #[serde(rename = "member")]
    name: String,
    entry: String,
    words: Vec<String>,
    form: String,
    /// Why the other SDKs that have its receiver lack it.
    #[serde(default)]
    only: Option<String>,
}

impl Member {
    /// Its words joined as a catalog name's, `start_crawl`.
    fn joined(&self) -> String {
        self.words.join("_")
    }

    /// Its package and receiver as every SDK names them, the line being
    /// `language`'s: `collie-client` for `@goodfetch/collie-client`,
    /// `goodfetch-collie-client` and `goodfetch_collie_client`, and
    /// `collie/testing` for `goodfetch_collie.testing`; `Run` for Rust's
    /// `blocking::Run` and Python's `AsyncRun` (ADR 0081).
    fn shared_receiver(&self, language: Language) -> (String, &str) {
        let package = match language {
            Language::Python => {
                let module = self
                    .package
                    .strip_prefix(PYTHON_PREFIX)
                    .unwrap_or(&self.package);
                let mut path = module.split('.');
                let first = path.next().unwrap_or_default().replace('_', "-");
                std::iter::once(first)
                    .chain(path.map(str::to_owned))
                    .collect::<Vec<_>>()
                    .join("/")
            }
            Language::TypeScript | Language::Rust => self
                .package
                .strip_prefix("@goodfetch/")
                .or_else(|| self.package.strip_prefix("goodfetch-"))
                .unwrap_or(&self.package)
                .to_owned(),
        };
        let receiver = match language {
            Language::Rust => self.receiver.strip_prefix(BLOCKING),
            Language::Python => self
                .receiver
                .strip_prefix(ASYNC)
                .filter(|name| name.starts_with(|first: char| first.is_ascii_uppercase())),
            Language::TypeScript => None,
        }
        .unwrap_or(&self.receiver);
        let receiver = if receiver == CRATE { MODULE } else { receiver };
        (package, receiver)
    }
}

struct Listed {
    path: &'static str,
    language: Language,
    members: Vec<Member>,
}

/// The manifest at `manifest`, relative to `root`.
fn read(root: &Path, manifest: &str) -> Result<Manifest> {
    let path = root.join(manifest);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn subject(path: &str, member: &Member) -> String {
    format!("{path} {}.{}", member.receiver, member.name)
}

/// How `language` spells `member` from its words and form, or `None` when
/// the language cannot spell them.
fn spelling(language: Language, member: &Member) -> Option<String> {
    let words = member.joined();
    if member.form == CONSTANT {
        return Some(words.to_uppercase());
    }
    match (language, member.form == TYPE) {
        (Language::TypeScript | Language::Python, true) => Some(codegen::pascal(&words)),
        (Language::TypeScript, false) => Some(codegen::camel(&words)),
        (Language::Rust, true) => codegen::rust_type(&words),
        (Language::Rust, false) => codegen::rust_snake(&words),
        (Language::Python, false) => Some(codegen::snake(&words)),
    }
}

/// Whether `member`, which serves no entry, is spelled from its words:
/// TypeScript's constructor from its receiver's, as `new Crawl(…)` is, its
/// disposal from `close`, as ADR 0078's scoped form is, and another
/// well-known symbol's member from that symbol's words, as
/// `[Symbol.asyncIterator]` is from `async iterator`.
fn spelled(language: Language, member: &Member) -> bool {
    match (language, member.name.as_str()) {
        (Language::TypeScript, "constructor") => {
            codegen::pascal(&member.joined()) == member.receiver
        }
        (Language::TypeScript, "[Symbol.asyncDispose]" | "[Symbol.dispose]") => {
            member.words == ["close"]
        }
        (Language::TypeScript, name) if name.starts_with("[Symbol.") => {
            name == format!("[Symbol.{}]", codegen::camel(&member.joined()))
        }
        _ => spelling(language, member).as_deref() == Some(member.name.as_str()),
    }
}

/// The member a generator writes, as `language` spells it.
fn generated_name(language: Language, words: &str) -> Option<String> {
    match language {
        Language::TypeScript => Some(codegen::camel(words)),
        Language::Rust => codegen::rust_snake(words),
        Language::Python => Some(codegen::snake(words)),
    }
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let catalog = catalog(root)?;
    let reach: BTreeMap<&str, Option<&str>> = catalog
        .operations
        .iter()
        .map(|operation| (operation.name.as_str(), operation.reach.as_deref()))
        .collect();
    let generated = codegen::handle_members(root)?;
    let mut listed = Vec::new();
    for (manifest, language) in MANIFESTS {
        listed.push(Listed {
            path: manifest,
            language,
            members: read(root, manifest)?.members,
        });
    }
    let mut findings = Vec::new();
    for manifest in &listed {
        check_entries(manifest, &reach, &generated, &mut findings);
        check_others(manifest, &listed, &mut findings);
    }
    Ok(findings)
}

/// The checks of one manifest's own lines: each entry, each spelling and
/// each receiver's entries.
fn check_entries(
    manifest: &Listed,
    reach: &BTreeMap<&str, Option<&str>>,
    generated: &[codegen::HandleMember],
    findings: &mut Vec<Finding>,
) {
    let mut served: BTreeSet<(&str, &str, &str)> = BTreeSet::new();
    for member in &manifest.members {
        let subject = subject(manifest.path, member);
        if !FORMS.contains(&member.form.as_str()) {
            findings.push(Finding::new(
                "unknown-form",
                subject,
                format!(
                    "`{}` is no form; a line declares one of {}",
                    member.form,
                    FORMS.join(", ")
                ),
            ));
            continue;
        }
        if member.entry == NO_ENTRY {
            if !spelled(manifest.language, member) {
                findings.push(Finding::new(
                    "misspelled",
                    subject,
                    format!(
                        "its words {:?} and form `{}` spell `{}` (ADR 0099)",
                        member.words,
                        member.form,
                        spelling(manifest.language, member).unwrap_or_default()
                    ),
                ));
            }
            continue;
        }
        match reach.get(member.entry.as_str()) {
            None => findings.push(Finding::new(
                "unknown-entry",
                subject.clone(),
                format!("`{}` is no operation of the catalog", member.entry),
            )),
            Some(Some(PERSON_ONLY)) => findings.push(Finding::new(
                "person-only",
                subject.clone(),
                format!(
                    "`{}` is only a person's to call (ADR 0041); no SDK member serves it",
                    member.entry
                ),
            )),
            Some(_) => {}
        }
        if !served.insert((&member.package, &member.receiver, &member.entry)) {
            findings.push(Finding::new(
                "duplicate-entry",
                subject.clone(),
                format!(
                    "`{}` already lists a member for `{}`; a receiver has one member per entry (ADR 0099)",
                    member.receiver, member.entry
                ),
            ));
        }
        let (_, receiver) = member.shared_receiver(manifest.language);
        let written = generated.iter().any(|written| {
            written.receiver == receiver
                && written.entry == member.entry
                && generated_name(manifest.language, &written.words).as_deref()
                    == Some(member.name.as_str())
        });
        if written {
            findings.push(Finding::new(
                "generated",
                subject,
                format!(
                    "the generator writes `{}` on `{receiver}` for `{}`; remove the hand-written member and its line (ADR 0099)",
                    member.name, member.entry
                ),
            ));
        }
    }
}

/// Fails each member of `manifest` whose words another SDK's manifest lacks
/// on the same receiver, unless its line says why.
fn check_others(manifest: &Listed, listed: &[Listed], findings: &mut Vec<Finding>) {
    for other in listed
        .iter()
        .filter(|other| other.language != manifest.language)
    {
        let mut words: BTreeMap<(String, &str), BTreeSet<&[String]>> = BTreeMap::new();
        for member in &other.members {
            words
                .entry(member.shared_receiver(other.language))
                .or_default()
                .insert(&member.words);
        }
        for member in manifest
            .members
            .iter()
            .filter(|member| member.only.is_none())
        {
            let shared = member.shared_receiver(manifest.language);
            let Some(held) = words.get(&shared) else {
                continue;
            };
            if !held.contains(member.words.as_slice()) {
                findings.push(Finding::new(
                    "missing-in-other-sdk",
                    subject(manifest.path, member),
                    format!(
                        "{} lists no member of the words {:?} on `{}`; add it there, or say why only this SDK has it in the line's `only` (ADR 0099)",
                        other.path,
                        member.words,
                        shared.1
                    ),
                ));
            }
        }
    }
}
