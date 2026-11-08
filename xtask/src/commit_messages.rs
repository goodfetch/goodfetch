//! The commit-message check (CONTRIBUTING.md): every commit a pull request
//! adds, from its base to its head, has a Conventional Commits subject,
//! `<type>[(<scope>)][!]: <description>`, which `xtask release` reads through
//! git-cliff to write the changelog. Commits that are ancestors of the
//! recorded starting point keep their messages, since history is never
//! rewritten, and merge commits are not judged, as the DCO check does not
//! judge them. Neither are the commits `ci/commit-messages.toml` lists, each
//! by its full hash with a one-line reason.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

/// The committed list of commits whose messages are not judged.
pub(crate) const EXCEPTIONS: &str = "ci/commit-messages.toml";

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Exceptions {
    #[serde(rename = "exception", default)]
    entries: Vec<Exception>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exception {
    /// The commit's full hash: 40 lowercase hexadecimal digits.
    commit: String,
    /// Why its message is kept, in one line.
    #[serde(default)]
    reason: String,
}

impl Exception {
    fn is_valid(&self) -> bool {
        let reason = self.reason.trim();
        self.commit.len() == 40
            && self
                .commit
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, 'a'..='f'))
            && !reason.is_empty()
            && !reason.contains('\n')
    }
}

/// The exceptions file, or none when the repository has no such file.
fn read_exceptions(root: &Path) -> Result<Exceptions> {
    let path = root.join(EXCEPTIONS);
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Exceptions::default()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// The types of the Conventional Commits specification's common set, as
/// `cliff.toml` groups or skips them.
const TYPES: [&str; 11] = [
    "build", "chore", "ci", "docs", "feat", "fix", "perf", "refactor", "revert", "style", "test",
];

pub(crate) fn check(root: &Path, base: &str, head: &str, since: &str) -> Result<Vec<Finding>> {
    let exceptions = read_exceptions(root)?;
    let mut findings: Vec<Finding> = exceptions
        .entries
        .iter()
        .filter(|entry| !entry.is_valid())
        .map(|entry| {
            Finding::new(
                "invalid-exception",
                &entry.commit,
                format!("an entry of {EXCEPTIONS} names a commit by its full hash, 40 lowercase hexadecimal digits, with a one-line `reason`"),
            )
        })
        .collect();
    let excepted: BTreeSet<&str> = exceptions
        .entries
        .iter()
        .map(|entry| entry.commit.as_str())
        .collect();
    let log = crate::git::text(
        root,
        &[
            "log",
            "--no-merges",
            "--format=%H %s",
            &format!("{base}..{head}"),
            "--not",
            since,
            "--",
        ],
    )?;
    findings.extend(log
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(id, subject)| !excepted.contains(id) && !is_conventional(subject))
        .map(|(id, subject)| {
            Finding::new(
                "unconventional-message",
                id,
                format!(
                    "`{subject}` is not `<type>[(<scope>)][!]: <description>`, with a type of {} (CONTRIBUTING.md)",
                    TYPES.join(", ")
                ),
            )
        }));
    Ok(findings)
}

/// Whether `subject` is `<type>[(<scope>)][!]: <description>`.
fn is_conventional(subject: &str) -> bool {
    let Some((prefix, description)) = subject.split_once(": ") else {
        return false;
    };
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let kind = match prefix.split_once('(') {
        Some((kind, scope)) => {
            let Some(scope) = scope.strip_suffix(')') else {
                return false;
            };
            let scope_ok = !scope.is_empty()
                && scope.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || matches!(character, '-' | '_' | '.' | '/')
                });
            if !scope_ok {
                return false;
            }
            kind
        }
        None => prefix,
    };
    TYPES.contains(&kind) && !description.trim().is_empty()
}
