//! The paths a change touches, read from git. A rename counts as a deletion
//! and an addition, so both folders are touched.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use super::RepoPath;

/// The branch whose merge base the local mode diffs against (ADR 0061),
/// unless `--since` names another.
const LOCAL_BASE: &str = "origin/main";

/// The changed paths of a diff, NUL-separated, renames split in two.
const DIFF_NAMES: [&str; 4] = ["diff", "--name-only", "--no-renames", "-z"];

/// Which change to read.
pub(crate) enum Range {
    /// CI's mode: what `head` changed since its merge base with `base`.
    Commits { base: String, head: String },
    /// The local mode: the working tree, uncommitted and untracked files
    /// included, against the merge base of `HEAD` with `since`, else with
    /// `origin/main`.
    WorkingTree { since: Option<String> },
}

pub(crate) fn changed_paths(root: &Path, range: &Range) -> Result<Vec<RepoPath>> {
    let mut paths = BTreeSet::new();
    match range {
        Range::Commits { base, head } => {
            let commits = format!("{base}...{head}");
            paths.extend(entries(&run(root, &DIFF_NAMES, &[&commits])?));
        }
        Range::WorkingTree { since } => {
            let since = since.as_deref().unwrap_or(LOCAL_BASE);
            let merge_base = run(root, &["merge-base"], &[since, "HEAD"])?;
            paths.extend(entries(&run(root, &DIFF_NAMES, &[merge_base.trim()])?));
            let untracked = ["ls-files", "--others", "--exclude-standard", "-z"];
            paths.extend(entries(&run(root, &untracked, &[])?));
        }
    }
    Ok(paths.into_iter().map(RepoPath::new).collect())
}

/// The entries of a `-z` output.
fn entries(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
}

/// Runs git with `args` then `operands` in `root`, and returns its output. An
/// operand is never read as an option.
fn run(root: &Path, args: &[&str], operands: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.args(args);
    if !operands.is_empty() {
        command.arg("--end-of-options").args(operands);
    }
    let output = command.current_dir(root).output().context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} {} failed: {}",
            args.join(" "),
            operands.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("reading git's output")
}
