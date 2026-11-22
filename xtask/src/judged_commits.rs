//! The commits a commit check judged green (ADR 0135's amendment of
//! 2026-11-22). A commit's sign-off and subject are part of the commit, so a
//! green verdict on it holds for as long as the check's rules do: the
//! pre-push hook judges each pushed commit, and `just quick`, `just
//! batch-check` and `just merge` judge only the commits no layer judged
//! green before, among them the merge of `main`'s tip that the gate makes.
//! A red verdict is never kept.
//!
//! The record is `judged-commits` in the gate's records
//! (`ci/gate-records.sh`), which every worktree shares: one `<check> <rules>
//! <commit>` line for each commit judged green, where `<rules>` is a digest
//! of what the verdict follows from beside the commit: the check's own source
//! and its inputs, such as `ci/commit-messages.toml`'s entries.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use sha2::{Digest as _, Sha256};

use crate::check::Finding;

/// The record's file in the gate's records.
const FILE: &str = "judged-commits";

/// A commit of a check's range, as `git log` lists it.
#[derive(Clone, Copy)]
pub(crate) struct Commit<'a> {
    pub(crate) id: &'a str,
    /// Whether it has more than one parent. A merge adds no line of its own,
    /// so the checks pass it, as the DCO app does.
    pub(crate) merge: bool,
}

impl<'a> Commit<'a> {
    /// Reads `<id> <parents>`, the parents separated by spaces.
    pub(crate) fn parse(id: &'a str, parents: &str) -> Self {
        Self {
            id,
            merge: parents.split_whitespace().count() > 1,
        }
    }
}

/// A commit check's verdict: the commits it did not judge again, as notes,
/// and its findings on the rest.
pub(crate) struct Judgment {
    pub(crate) notes: Vec<Finding>,
    pub(crate) findings: Vec<Finding>,
}

impl Judgment {
    pub(crate) fn report(&self) -> std::process::ExitCode {
        crate::check::note(&self.notes);
        crate::check::report(&self.findings)
    }
}

/// The commits one check judged green on its present rules.
pub(crate) struct JudgedCommits {
    /// The record's file, or none when the caller keeps no record.
    file: Option<PathBuf>,
    check: &'static str,
    rules: String,
    green: BTreeSet<String>,
}

impl JudgedCommits {
    /// The commits `check` judged green, read from the gate's records
    /// `records` (none keeps no record), on the rules whose parts are
    /// `rules`.
    pub(crate) fn read(
        records: Option<&Path>,
        check: &'static str,
        rules: &[&[u8]],
    ) -> Result<Self> {
        let mut digest = Sha256::new();
        for part in rules {
            digest.update(part.len().to_le_bytes());
            digest.update(part);
        }
        let rules = format!("{:x}", digest.finalize());
        let file = records.map(|records| records.join(FILE));
        let text = match &file {
            None => String::new(),
            Some(file) => match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", file.display()));
                }
            },
        };
        let green = text
            .lines()
            .filter_map(|line| {
                let mut fields = line.split(' ');
                match (fields.next(), fields.next(), fields.next(), fields.next()) {
                    (Some(name), Some(digest), Some(commit), None)
                        if name == check && digest == rules =>
                    {
                        Some(commit.to_owned())
                    }
                    _ => None,
                }
            })
            .collect();
        Ok(Self {
            file,
            check,
            rules,
            green,
        })
    }

    /// The commits of `commits` judged green before, each as a note that
    /// says it is not judged again, and those left to judge.
    pub(crate) fn split<'a, 'b>(
        &self,
        commits: &'b [Commit<'a>],
    ) -> (Vec<Finding>, Vec<&'b Commit<'a>>) {
        let (before, left): (Vec<&Commit<'_>>, Vec<&Commit<'_>>) = commits
            .iter()
            .partition(|commit| self.green.contains(commit.id));
        let notes = before
            .into_iter()
            .map(|commit| {
                Finding::new(
                    "judged-green-before",
                    commit.id,
                    format!(
                        "a layer judged it green before on the same rules (`{FILE}` in the gate's records), so `{}` does not judge it again",
                        self.check
                    ),
                )
            })
            .collect();
        (notes, left)
    }

    /// Records as green each commit of `judged` that no finding names.
    pub(crate) fn add(&self, judged: &[&Commit<'_>], findings: &[Finding]) -> Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let red: BTreeSet<&str> = findings
            .iter()
            .map(|finding| finding.subject.as_str())
            .collect();
        let lines: String = judged
            .iter()
            .filter(|commit| !red.contains(commit.id))
            .map(|commit| format!("{} {} {}\n", self.check, self.rules, commit.id))
            .collect();
        if lines.is_empty() {
            return Ok(());
        }
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)
                .with_context(|| format!("creating {}", folder.display()))?;
        }
        // One append of whole lines, so layers that record at once never
        // interleave a line.
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .and_then(|mut opened| opened.write_all(lines.as_bytes()))
            .with_context(|| format!("writing {}", file.display()))
    }
}
