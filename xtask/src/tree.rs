//! The repository's own files, walked from a folder: never version control,
//! hidden folders, build outputs, installed packages, fixtures (samples the
//! checks run on purpose) or the frozen `rnd/` archive (ADR 0032).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

/// Folders never entered, wherever they are.
const SKIPPED: [&str; 5] = ["target", "node_modules", "fixtures", "dist", "__pycache__"];

/// The frozen R&D archive at the root (ADR 0032).
const FROZEN: &str = "rnd";

/// Every file under `folder` (relative to `root`), as sorted `/`-separated
/// paths relative to `root`; nothing when `folder` does not exist.
pub(crate) fn files(root: &Path, folder: &str) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let start = root.join(folder);
    if start.is_dir() {
        walk(root, &start, &mut files)?;
    }
    files.sort();
    Ok(files)
}

fn walk(root: &Path, folder: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry
            .file_type()
            .with_context(|| format!("reading {}", path.display()))?;
        if kind.is_dir() {
            let frozen = folder == root && name == FROZEN;
            if !(name.starts_with('.') || SKIPPED.contains(&name.as_ref()) || frozen) {
                walk(root, &path, files)?;
            }
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
