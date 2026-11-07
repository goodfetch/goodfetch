//! The saved size report: every measured file of one build, keyed by the
//! Git tree that was built (ADR 0061). The merge-queue run whose tree became
//! `main`'s head saved `main`'s report (a manual-dispatch Release run on
//! `main` while the repository is private), so a report is `main`'s only
//! when its tree is the one `main`'s head has.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use super::budget::Bytes;
use super::files::{Measured, Profile};

/// A Git tree's object name.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Tree(String);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub(crate) tree: Tree,
    pub(crate) profile: Profile,
    pub(crate) files: Vec<Measured>,
}

/// The tree of `reference`, a commit-ish of the repository at `root`.
pub(crate) fn tree(root: &Path, reference: &str) -> Result<Tree> {
    let tree = format!("{reference}^{{tree}}");
    match crate::git::line(root, &["rev-parse", "--verify", "--quiet", &tree]) {
        Ok(name) => Ok(Tree(name)),
        Err(crate::git::Error::Refused { .. }) => {
            bail!("git cannot resolve the tree of `{reference}`")
        }
        Err(error) => Err(error.into()),
    }
}

/// Saves the report of `files`, keyed by the tree of the workspace's `HEAD`,
/// the tree that was built.
pub(crate) fn save(root: &Path, path: &Path, profile: &Profile, files: &[Measured]) -> Result<()> {
    let report = Report {
        tree: tree(root, "HEAD")?,
        profile: profile.clone(),
        files: files.to_vec(),
    };
    let path = root.join(path);
    let text = serde_json::to_string_pretty(&report).context("serializing the size report")?;
    std::fs::write(&path, text + "\n").with_context(|| format!("writing {}", path.display()))
}

/// Reads `main`'s report and returns its figure per file identifier. It
/// refuses a report saved for another tree than `main`'s head's, or for
/// another profile than the files measured now.
pub(crate) fn read_main(
    root: &Path,
    path: &Path,
    main: &str,
    profile: &Profile,
) -> Result<BTreeMap<String, Bytes>> {
    let path = root.join(path);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let report: Report =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let expected = tree(root, main)?;
    if report.tree != expected {
        bail!(
            "{} was saved for tree {}, not for `{main}`'s tree {}",
            path.display(),
            report.tree.0,
            expected.0
        );
    }
    if report.profile != *profile {
        bail!(
            "{} measured the `{}` profile, not `{}`",
            path.display(),
            report.profile.0,
            profile.0
        );
    }
    Ok(report
        .files
        .iter()
        .map(|file| (file.id(), file.bytes))
        .collect())
}
