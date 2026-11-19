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
//! `@goodfetch/` or `goodfetch-` prefix aside; a Rust `blocking::` twin is
//! its async receiver, and a Rust crate's root is a TypeScript module's.
//! Matching a manifest to the members an SDK exports, their doc text and
//! markers, and the Python SDK's manifest are issue 2920's.
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

/// The catalog `xtask codegen` emits from `goodfetch-protocol`'s types.
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";
/// Each SDK's manifest and its language; an SDK that lists its hand-written
/// members adds its own here.
const MANIFESTS: [(&str, Language); 2] = [
    ("sdks/typescript/manifest.toml", Language::TypeScript),
    (
        "crates/foxhound/goodfetch-foxhound/manifest.toml",
        Language::Rust,
    ),
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
/// The receiver of a member at a TypeScript module's top level; a Rust
/// crate's root is `(crate)`.
const MODULE: &str = "(module)";
const CRATE: &str = "(crate)";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    TypeScript,
    Rust,
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
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "member", default)]
    members: Vec<Member>,
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

    /// Its receiver and package as every SDK names them.
    fn shared_receiver(&self) -> (&str, &str) {
        let package = self
            .package
            .strip_prefix("@goodfetch/")
            .or_else(|| self.package.strip_prefix("goodfetch-"))
            .unwrap_or(&self.package);
        let receiver = self
            .receiver
            .strip_prefix(BLOCKING)
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
        (Language::TypeScript, true) => Some(codegen::class_name(&words)),
        (Language::TypeScript, false) => Some(codegen::typescript_member(&words)),
        (Language::Rust, true) => codegen::rust_type(&words),
        (Language::Rust, false) => codegen::rust_member(&words),
    }
}

/// Whether `member`, which serves no entry, is spelled from its words:
/// TypeScript's constructor from its receiver's, as `new Crawl(…)` is, and
/// its disposal from `close`, as ADR 0078's scoped form is.
fn spelled(language: Language, member: &Member) -> bool {
    match (language, member.name.as_str()) {
        (Language::TypeScript, "constructor") => {
            codegen::class_name(&member.joined()) == member.receiver
        }
        (Language::TypeScript, "[Symbol.asyncDispose]" | "[Symbol.dispose]") => {
            member.words == ["close"]
        }
        _ => spelling(language, member).as_deref() == Some(member.name.as_str()),
    }
}

/// The member a generator writes, as `language` spells it.
fn generated_name(language: Language, words: &str) -> Option<String> {
    match language {
        Language::TypeScript => Some(codegen::typescript_member(words)),
        Language::Rust => codegen::rust_member(words),
    }
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let path = root.join(CATALOG);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let catalog: Catalog =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let reach: BTreeMap<&str, Option<&str>> = catalog
        .operations
        .iter()
        .map(|operation| (operation.name.as_str(), operation.reach.as_deref()))
        .collect();
    let generated = codegen::handle_members(root)?;
    let mut listed = Vec::new();
    for (manifest, language) in MANIFESTS {
        let path = root.join(manifest);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: Manifest =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        listed.push(Listed {
            path: manifest,
            language,
            members: parsed.members,
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
        let (_, receiver) = member.shared_receiver();
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
        let mut words: BTreeMap<(&str, &str), BTreeSet<&[String]>> = BTreeMap::new();
        for member in &other.members {
            words
                .entry(member.shared_receiver())
                .or_default()
                .insert(&member.words);
        }
        for member in manifest
            .members
            .iter()
            .filter(|member| member.only.is_none())
        {
            let Some(held) = words.get(&member.shared_receiver()) else {
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
                        member.shared_receiver().1
                    ),
                ));
            }
        }
    }
}
