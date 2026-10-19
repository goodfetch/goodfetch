//! The commit-message check (CONTRIBUTING.md): every commit a pull request
//! adds, from its base to its head, has a Conventional Commits subject,
//! `<type>[(<scope>)][!]: <description>`, which `xtask release` reads through
//! git-cliff to write the changelog. Commits that are ancestors of the
//! recorded starting point keep their messages, since history is never
//! rewritten, and merge commits are not judged, as the DCO check does not
//! judge them.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use crate::check::Finding;

/// The types of the Conventional Commits specification's common set, as
/// `cliff.toml` groups or skips them.
const TYPES: [&str; 11] = [
    "build", "chore", "ci", "docs", "feat", "fix", "perf", "refactor", "revert", "style", "test",
];

pub(crate) fn check(root: &Path, base: &str, head: &str, since: &str) -> Result<Vec<Finding>> {
    let output = Command::new("git")
        .args(["log", "--no-merges", "--format=%H %s"])
        .arg(format!("{base}..{head}"))
        .arg("--not")
        .arg(since)
        .arg("--")
        .current_dir(root)
        .output()
        .context("running git log")?;
    if !output.status.success() {
        bail!(
            "git log {base}..{head} --not {since} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let log = String::from_utf8(output.stdout).context("reading git log's output")?;
    Ok(log
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(_, subject)| !is_conventional(subject))
        .map(|(id, subject)| {
            Finding::new(
                "unconventional-message",
                id,
                format!(
                    "`{subject}` is not `<type>[(<scope>)][!]: <description>`, with a type of {} (CONTRIBUTING.md)",
                    TYPES.join(", ")
                ),
            )
        })
        .collect())
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
