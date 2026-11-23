//! What the API breakage checks compare with and how they judge what they
//! find (ADR 0030, issue 148): `compat/api.toml`'s previous release's
//! commit and the breaks it announces for the next major. `xtask api-check`
//! compares the contracts with that commit, `xtask semver-checks` the Rust
//! crates under semver, and `xtask sdk-api` the TypeScript and Python SDK
//! packages; each judges the announcements of its own subjects, so an
//! announcement of another's is never one without a break.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::git;

/// The previous release's commit and the announced breaks.
const API: &str = "compat/api.toml";

/// How the subject of a Rust crate's break starts: `xtask semver-checks`
/// judges those announcements.
pub(crate) const RUST: &str = "rust ";
/// How the subject of a TypeScript or Python package's break starts:
/// `xtask sdk-api` judges those announcements.
pub(crate) const TYPESCRIPT: &str = "typescript ";
pub(crate) const PYTHON: &str = "python ";

/// Whether `entry` is a package's break, which `xtask semver-checks` or
/// `xtask sdk-api` judges; `xtask api-check` judges every other.
pub(crate) fn is_package_break(entry: &str) -> bool {
    [RUST, TYPESCRIPT, PYTHON]
        .iter()
        .any(|prefix| entry.starts_with(prefix))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Api {
    /// The commit that stands for the previous release of the major.
    previous: String,
    #[serde(default)]
    announced: Vec<Announced>,
}

/// A break announced for the next major.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Announced {
    /// The finding's subject: `<collie|foxhound> <section> <name>`,
    /// `bundle outputs <command>` or `bundle command_failed` for the
    /// contracts, `rust <crate> <lint>` for a crate's ([`RUST`]),
    /// `typescript <entry> <name>` and `python <module> <name>` for an SDK
    /// package's ([`TYPESCRIPT`], [`PYTHON`]).
    entry: String,
    /// Why, as the release notes will say it.
    reason: String,
}

/// What an API breakage check compares with: the previous release's commit
/// and the breaks announced for the next major, from [`API`].
pub(crate) struct Baseline {
    /// The previous release's commit, its full name.
    pub(crate) commit: String,
    announced: Vec<Announced>,
}

/// Reads [`API`] and resolves its previous release's commit.
pub(crate) fn baseline(root: &Path) -> Result<Baseline> {
    let text = std::fs::read_to_string(root.join(API)).with_context(|| format!("reading {API}"))?;
    let api: Api = toml::from_str(&text).with_context(|| format!("parsing {API}"))?;
    let commit = git::commit(root, &api.previous)
        .with_context(|| format!("resolving {API}'s previous release `{}`", api.previous))?;
    Ok(Baseline {
        commit,
        announced: api.announced,
    })
}

/// Writes the files of `commit` under `paths` into the new folder `into`,
/// with `git archive`: the whole tree when `paths` is empty, and only the
/// paths the commit has otherwise.
pub(crate) fn archive(root: &Path, commit: &str, paths: &[&str], into: &Path) -> Result<()> {
    std::fs::create_dir_all(into).with_context(|| format!("creating {}", into.display()))?;
    let mut listing = vec!["ls-tree", "--name-only", commit, "--"];
    listing.extend(paths);
    let present = if paths.is_empty() {
        Vec::new()
    } else {
        let listed = git::text(root, &listing)
            .with_context(|| format!("listing {} in {commit}", paths.join(", ")))?;
        let present: Vec<String> = listed.lines().map(str::to_owned).collect();
        if present.is_empty() {
            return Ok(());
        }
        present
    };
    let mut archive = git::command(root)
        .args(["archive", "--format=tar", commit, "--"])
        .args(&present)
        .stdout(Stdio::piped())
        .spawn()
        .context("running git archive")?;
    let tar = archive
        .stdout
        .take()
        .context("invariant: git archive's output is piped")?;
    let extracted = std::process::Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(into)
        .stdin(tar)
        .status()
        .context("running tar")?;
    let archived = archive.wait().context("waiting for git archive")?;
    if !archived.success() || !extracted.success() {
        bail!(
            "writing the tree of {commit} under {} failed",
            into.display()
        );
    }
    Ok(())
}

impl Baseline {
    /// Judges the changes of each entry against the announcements whose
    /// subject `owns` gives to the calling check: a break fails unless
    /// announced, an announcement fails without a reason or without a break
    /// to pass, and additions and announced breaks are notes.
    pub(crate) fn verdict(
        &self,
        owns: impl Fn(&str) -> bool,
        entries: &BTreeMap<String, Changes>,
    ) -> Outcome {
        let owned: Vec<&Announced> = self
            .announced
            .iter()
            .filter(|announcement| owns(&announcement.entry))
            .collect();
        let mut announced: BTreeMap<&str, &Announced> = owned
            .iter()
            .map(|announcement| (announcement.entry.as_str(), *announcement))
            .collect();
        let mut findings: Vec<Finding> = owned
            .iter()
            .filter(|announcement| announcement.reason.trim().is_empty())
            .map(|announcement| {
                Finding::new(
                    "announcement-without-reason",
                    &announcement.entry,
                    "an announced break says why, for the release notes",
                )
            })
            .collect();
        let mut notes = Vec::new();
        for (entry, changes) in entries {
            if !changes.breaks.is_empty() {
                let breaks = changes.breaks.join("; ");
                match announced.remove(entry.as_str()) {
                    Some(announcement) => notes.push(Finding::new(
                        "announced-break",
                        entry,
                        format!("{breaks} (announced: {})", announcement.reason),
                    )),
                    None => findings.push(Finding::new(
                        "unannounced-break",
                        entry,
                        format!(
                            "{breaks}; a break waits for the next major: announce it in {API}, or keep the entry compatible"
                        ),
                    )),
                }
            } else if !changes.additions.is_empty() {
                notes.push(Finding::new(
                    "addition",
                    entry,
                    changes.additions.join("; "),
                ));
            }
        }
        for entry in announced.keys() {
            findings.push(Finding::new(
                "announced-without-break",
                *entry,
                format!("{API} announces a break that the comparison with the previous release no longer finds"),
            ));
        }
        Outcome { findings, notes }
    }
}

/// What changed in one entry since the previous release.
#[derive(Default)]
pub(crate) struct Changes {
    breaks: Vec<String>,
    additions: Vec<String>,
}

impl Changes {
    pub(crate) fn breaks(&mut self, at: &str, what: impl std::fmt::Display) {
        self.breaks.push(format!("{at}: {what}"));
    }

    pub(crate) fn adds(&mut self, at: &str, what: impl std::fmt::Display) {
        self.additions.push(format!("{at}: {what}"));
    }

    /// Whether nothing changed.
    pub(crate) fn is_empty(&self) -> bool {
        self.breaks.is_empty() && self.additions.is_empty()
    }
}

/// What the comparison found: the breaks that fail and the notes beside them.
pub(crate) struct Outcome {
    pub(crate) findings: Vec<Finding>,
    pub(crate) notes: Vec<Finding>,
}

impl Outcome {
    /// Writes the notes, then the findings, and returns the exit code.
    pub(crate) fn report(&self) -> ExitCode {
        check::note(&self.notes);
        check::report(&self.findings)
    }
}
