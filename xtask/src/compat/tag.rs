//! A release tag's files, read through Git without checking the tag out:
//! `freeze` copies each format's `head/` set from the tag, never from the
//! working tree, which may hold newer, unreleased versions (ADR 0066).
use std::path::Path;

use crate::git;
use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::releases::ProductVersion;

use super::tree::Tree;

/// What a tag names.
pub(super) enum Release {
    /// A stable release or a patch, `vX.Y.Z`.
    Stable,
    /// A release candidate or a nightly, `vX.Y.Z-…`.
    Prerelease,
}

/// What the tag `vX.Y.Z[-…]` names.
pub(super) fn release(tag: &str) -> Result<Release> {
    let version = tag
        .strip_prefix('v')
        .with_context(|| format!("`{tag}` is no release tag `vX.Y.Z`"))?;
    let (core, prerelease) = match version.split_once('-') {
        Some((core, suffix)) if !suffix.is_empty() => (core, true),
        Some(_) => bail!("`{tag}` is no release tag `vX.Y.Z`"),
        None => (version, false),
    };
    ProductVersion::try_from(core.to_owned())
        .with_context(|| format!("`{tag}` is no release tag `vX.Y.Z`"))?;
    Ok(if prerelease {
        Release::Prerelease
    } else {
        Release::Stable
    })
}

/// Fails unless `tag` names a commit of the repository at `root`.
pub(super) fn verify(root: &Path, tag: &str) -> Result<()> {
    git::bytes(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tag}^{{commit}}"),
        ],
    )
    .with_context(|| format!("the tag `{tag}` names no commit"))?;
    Ok(())
}

/// One file as the tag holds it.
pub(super) fn file(root: &Path, tag: &str, path: &str) -> Result<Vec<u8>> {
    git::bytes(root, &["cat-file", "blob", &format!("{tag}:{path}")])
        .with_context(|| format!("reading {path} at {tag}"))
}

/// Every file the tag holds under `folder`, by its path inside it.
pub(super) fn folder(root: &Path, tag: &str, folder: &str) -> Result<Tree> {
    let prefix = format!("{folder}/");
    let listing = git::bytes(
        root,
        &["ls-tree", "-r", "-z", "--name-only", tag, "--", &prefix],
    )?;
    let mut tree = Tree::new();
    for path in listing
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(path).context("reading a path git listed")?;
        let relative = path.strip_prefix(&prefix).unwrap_or(path);
        tree.insert(relative.to_owned(), file(root, tag, path)?);
    }
    Ok(tree)
}
