//! A set's files as one value: each file's path inside its folder, written
//! with `/`, and its bytes.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};

/// Files by their path inside a folder.
pub(super) type Tree = BTreeMap<String, Vec<u8>>;

/// Files the operating system leaves beside others, which are never a set's.
const IGNORED: [&str; 1] = [".DS_Store"];

/// Every file under `folder`; none when it does not exist.
pub(super) fn read(folder: &Path) -> Result<Tree> {
    let mut tree = Tree::new();
    if !folder.is_dir() {
        return Ok(tree);
    }
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = std::fs::read_dir(&current)
            .with_context(|| format!("reading {}", current.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("reading {}", current.display()))?;
            if IGNORED.contains(&entry.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                let bytes =
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let relative = path.strip_prefix(folder).unwrap_or(&path);
                tree.insert(relative.to_string_lossy().replace('\\', "/"), bytes);
            }
        }
    }
    Ok(tree)
}

/// Makes `folder` hold exactly `tree`.
pub(super) fn replace(folder: &Path, tree: &Tree) -> Result<()> {
    if folder.exists() {
        std::fs::remove_dir_all(folder)
            .with_context(|| format!("removing {}", folder.display()))?;
    }
    add(folder, tree)
}

/// Writes every file of `tree` into `folder`.
pub(super) fn add(folder: &Path, tree: &Tree) -> Result<()> {
    for (relative, bytes) in tree {
        let path = folder.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}
