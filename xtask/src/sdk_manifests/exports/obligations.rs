//! The doc obligations (ADRs 0117, 0119, 0121): the rules users missed,
//! each listed once in [`OBLIGATIONS`] with the texts that state it. An
//! obligation's text is in each text it binds, however that text wraps its
//! lines, and at its start when it `opens` it:
//! - `members`: the hand-written members each SDK's manifest lists, chosen
//!   by their entry, their receiver as every SDK names it and their words
//!   joined by `_`, each with the doc text [`super::exports`] reads;
//! - `operations` and `settings`: the catalog's description, which the
//!   generator copies into every SDK's doc text and an MCP tool's
//!   description (ADR 0038);
//! - `schemas`: a protocol type's description in the JSON Schema bundle,
//!   which the generator copies the same way;
//! - `pages`: a docs page.
//!
//! A `pending` obligation names operations and settings that do not exist
//! yet, and fails once one does, so that it binds it from then on. Each
//! obligation's text, each text it binds and each operation's description
//! is English only (ADR 0073), and an operation's description holds at most
//! 2,048 characters.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Exported, flattened, listed_export};
use crate::check::Finding;
use crate::sdk_manifests::{CATALOG, Catalog, Language, Member};

/// Where the obligations are listed.
const OBLIGATIONS: &str = "sdks/doc-obligations.toml";
/// The JSON Schema bundle `xtask codegen` writes.
const BUNDLE: &str = "crates/goodfetch-protocol/generated/bundle.json";
/// The most characters an operation's description holds, so that an MCP
/// tool's description, which copies it, stays whole in every client.
const LONGEST: usize = 2_048;

/// The members one SDK's manifest lists, and what its packages export.
pub(super) struct Sdk<'a> {
    pub(super) manifest: &'a str,
    pub(super) language: Language,
    pub(super) members: &'a [Member],
    pub(super) exported: &'a [Exported],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Obligations {
    #[serde(rename = "obligation")]
    obligations: Vec<Obligation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Obligation {
    text: String,
    /// Whether each text it binds starts with it.
    #[serde(default)]
    opens: bool,
    #[serde(default)]
    members: Vec<Selector>,
    #[serde(default)]
    operations: Vec<String>,
    #[serde(default)]
    settings: Vec<String>,
    #[serde(default)]
    schemas: Vec<String>,
    #[serde(default)]
    pages: Vec<String>,
    /// Why the operations and settings it names do not exist yet.
    #[serde(default)]
    pending: Option<String>,
}

/// The manifest lines an obligation binds: each that matches every field
/// given.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selector {
    entry: Option<String>,
    /// As every SDK names it: `Run` for Rust's `blocking::Run` and Python's
    /// `AsyncRun`.
    receiver: Option<String>,
    /// Its words joined by `_`, `aria_snapshot` for TypeScript's
    /// `ariaSnapshot`.
    member: Option<String>,
}

impl Selector {
    fn chooses(&self, language: Language, member: &Member) -> bool {
        let (_, receiver) = member.shared_receiver(language);
        self.entry
            .as_deref()
            .is_none_or(|entry| entry == member.entry)
            && self.receiver.as_deref().is_none_or(|name| name == receiver)
            && self
                .member
                .as_deref()
                .is_none_or(|name| name == member.joined())
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(entry) = &self.entry {
            parts.push(format!("entry {entry}"));
        }
        match (&self.receiver, &self.member) {
            (Some(receiver), Some(member)) => parts.push(format!("{receiver}.{member}")),
            (Some(receiver), None) => parts.push(format!("{receiver}.*")),
            (None, Some(member)) => parts.push(format!("*.{member}")),
            (None, None) => {}
        }
        formatter.write_str(&parts.join(" "))
    }
}

pub(super) fn check(
    root: &Path,
    catalog: &Catalog,
    sdks: &[Sdk<'_>],
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let obligations = read(root)?;
    let operations: BTreeMap<&str, &str> = catalog
        .operations
        .iter()
        .map(|operation| (operation.name.as_str(), operation.description.as_str()))
        .collect();
    let settings: BTreeMap<&str, &str> = catalog
        .settings
        .iter()
        .map(|setting| (setting.name.as_str(), setting.description.as_str()))
        .collect();
    let schemas = schemas(root)?;
    let schemas: BTreeMap<&str, &str> = schemas
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect();
    let mut read_once = BTreeSet::new();
    for (index, obligation) in obligations.iter().enumerate() {
        let name = format!("obligation {}", index + 1);
        let text = flattened(&obligation.text);
        english(
            &format!("{OBLIGATIONS} {name}"),
            &text,
            &mut read_once,
            findings,
        );
        if obligation.pending.is_some() {
            pending(obligation, &text, &operations, &settings, findings);
            continue;
        }
        let mut texts = Vec::new();
        let mut unbound = Vec::new();
        bind(
            &obligation.operations,
            &operations,
            CATALOG,
            &mut texts,
            &mut unbound,
        );
        bind(
            &obligation.settings,
            &settings,
            CATALOG,
            &mut texts,
            &mut unbound,
        );
        bind(
            &obligation.schemas,
            &schemas,
            BUNDLE,
            &mut texts,
            &mut unbound,
        );
        for page in &obligation.pages {
            match std::fs::read_to_string(root.join(page)) {
                Ok(doc) => texts.push((page.clone(), doc)),
                Err(error) if error.kind() == ErrorKind::NotFound => unbound.push(page.clone()),
                Err(error) => return Err(error).with_context(|| format!("reading {page}")),
            }
        }
        members(&obligation.members, sdks, &mut texts, &mut unbound);
        if texts.is_empty() && unbound.is_empty() {
            unbound.push(name);
        }
        for bound in unbound {
            findings.push(Finding::new(
                "unbound",
                format!("{OBLIGATIONS} {bound}"),
                format!(
                    "the obligation \"{text}\" binds `{bound}`, which does not exist; correct it, or make the obligation `pending` with its reason"
                ),
            ));
        }
        for (subject, doc) in texts {
            let doc = flattened(&doc);
            let (holds, how) = if obligation.opens {
                (doc.starts_with(&text), "does not open with")
            } else {
                (doc.contains(&text), "lacks")
            };
            if !holds {
                findings.push(Finding::new(
                    "missing-obligation",
                    subject.clone(),
                    format!("its doc {how} \"{text}\" ({OBLIGATIONS}; ADRs 0117, 0121)"),
                ));
            }
            english(&subject, &doc, &mut read_once, findings);
        }
    }
    descriptions(catalog, &mut read_once, findings);
    Ok(())
}

/// Fails each operation or setting a pending obligation names that exists
/// now.
fn pending(
    obligation: &Obligation,
    text: &str,
    operations: &BTreeMap<&str, &str>,
    settings: &BTreeMap<&str, &str>,
    findings: &mut Vec<Finding>,
) {
    let existing = obligation
        .operations
        .iter()
        .filter(|entry| operations.contains_key(entry.as_str()))
        .chain(
            obligation
                .settings
                .iter()
                .filter(|entry| settings.contains_key(entry.as_str())),
        );
    for entry in existing {
        findings.push(Finding::new(
            "pending-bound",
            format!("{OBLIGATIONS} {entry}"),
            format!(
                "`{entry}` exists now; remove the obligation's `pending`, so that it binds it, and give its description \"{text}\""
            ),
        ));
    }
}

/// The doc of each listed member a selector chooses, in every SDK, as a
/// text to read, or the selector as unbound when it chooses none.
fn members(
    selectors: &[Selector],
    sdks: &[Sdk<'_>],
    texts: &mut Vec<(String, String)>,
    unbound: &mut Vec<String>,
) {
    for selector in selectors {
        let mut chosen = false;
        for sdk in sdks {
            for member in sdk
                .members
                .iter()
                .filter(|member| selector.chooses(sdk.language, member))
            {
                chosen = true;
                if let Some(found) = listed_export(member, sdk.exported) {
                    texts.push((
                        crate::sdk_manifests::subject(sdk.manifest, member),
                        found.doc.clone(),
                    ));
                }
            }
        }
        if !chosen {
            unbound.push(selector.to_string());
        }
    }
}

/// Fails each operation's description past [`LONGEST`] characters or
/// holding a letter English does not write.
fn descriptions(catalog: &Catalog, read_once: &mut BTreeSet<String>, findings: &mut Vec<Finding>) {
    for operation in &catalog.operations {
        let subject = format!("{CATALOG} {}", operation.name);
        let length = operation.description.chars().count();
        if length > LONGEST {
            findings.push(Finding::new(
                "too-long",
                subject.clone(),
                format!(
                    "its description has {length} characters, past the {LONGEST} an MCP tool's description holds; move a detail to the parameter or type it is about"
                ),
            ));
        }
        english(&subject, &operation.description, read_once, findings);
    }
}

/// Each of `names` in `known`, as a text to read, or as unbound.
fn bind(
    names: &[String],
    known: &BTreeMap<&str, &str>,
    file: &str,
    texts: &mut Vec<(String, String)>,
    unbound: &mut Vec<String>,
) {
    for name in names {
        match known.get(name.as_str()) {
            Some(description) => texts.push((format!("{file} {name}"), (*description).to_owned())),
            None => unbound.push(name.clone()),
        }
    }
}

/// Fails `text` when it holds a letter English does not write, once for
/// each `subject`.
fn english(
    subject: &str,
    text: &str,
    read_once: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    if !read_once.insert(subject.to_owned()) {
        return;
    }
    if let Some(letter) = text
        .chars()
        .find(|character| character.is_alphabetic() && !character.is_ascii())
    {
        findings.push(Finding::new(
            "not-english",
            subject,
            format!("it holds `{letter}`, a letter English does not write: V1's texts are in English (ADR 0073)"),
        ));
    }
}

/// The obligations, which each run of the check reads.
fn read(root: &Path) -> Result<Vec<Obligation>> {
    let path = root.join(OBLIGATIONS);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {OBLIGATIONS}"))?;
    let read: Obligations =
        toml::from_str(&text).with_context(|| format!("parsing {OBLIGATIONS}"))?;
    for obligation in &read.obligations {
        if obligation.members.iter().any(|selector| {
            selector.entry.is_none() && selector.receiver.is_none() && selector.member.is_none()
        }) {
            bail!(
                "{OBLIGATIONS}: the obligation \"{}\" chooses members by none of `entry`, `receiver` and `member`",
                obligation.text
            );
        }
        if obligation.pending.is_some()
            && !(obligation.members.is_empty()
                && obligation.schemas.is_empty()
                && obligation.pages.is_empty())
        {
            bail!(
                "{OBLIGATIONS}: the pending obligation \"{}\" names members, schemas or pages, which exist; a pending one names only operations and settings",
                obligation.text
            );
        }
    }
    Ok(read.obligations)
}

/// Each type's description in the JSON Schema bundle, by its name.
fn schemas(root: &Path) -> Result<BTreeMap<String, String>> {
    #[derive(Deserialize)]
    struct Bundle {
        #[serde(rename = "$defs", default)]
        defs: BTreeMap<String, serde_json::Value>,
    }
    let path = root.join(BUNDLE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {BUNDLE}"))?;
    let bundle: Bundle =
        serde_json::from_str(&text).with_context(|| format!("parsing {BUNDLE}"))?;
    Ok(bundle
        .defs
        .into_iter()
        .map(|(name, schema)| {
            let description = schema
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            (name, description)
        })
        .collect())
}
