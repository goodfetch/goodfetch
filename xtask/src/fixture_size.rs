//! The fixture-size check (ADR 0062): no file under a `fixtures/` folder
//! exceeds 256 KiB. Reference screenshots live outside `fixtures/` folders
//! and keep their own limit (ADR 0073); frozen `rnd/` is never read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::check::Finding;

/// TESTING.md, Fixtures and data files: 256 KiB.
const LIMIT_BYTES: u64 = 256 * 1024;

/// Folders the walk never enters, wherever they are: version control, build
/// outputs and installed packages.
const SKIPPED: [&str; 4] = [".git", "target", "node_modules", ".venv"];

/// The frozen R&D archive at the root (ADR 0032).
const FROZEN: &str = "rnd";

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    walk(root, root, false, &mut findings)?;
    Ok(findings)
}

fn walk(root: &Path, folder: &Path, in_fixtures: bool, findings: &mut Vec<Finding>) -> Result<()> {
    let mut entries = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if metadata.is_dir() {
            if SKIPPED.contains(&name) || (folder == root && name == FROZEN) {
                continue;
            }
            walk(root, &path, in_fixtures || name == "fixtures", findings)?;
        } else if in_fixtures && metadata.len() > LIMIT_BYTES {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            findings.push(Finding::new(
                "oversized-fixture",
                relative.to_string_lossy().replace('\\', "/"),
                format!(
                    "{} bytes is over the 256 KiB fixture limit; generate it from a seed with a `.source.toml` beside it",
                    metadata.len()
                ),
            ));
        }
    }
    Ok(())
}
