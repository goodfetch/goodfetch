//! The fixture-size check (ADR 0062): no file under a `fixtures/` folder, or
//! under `ci/proofs/`, which holds the static checks' fixtures, exceeds 256
//! KiB. A Reference screenshot is not a fixture, wherever it sits, and keeps
//! its own limit (ADR 0073); frozen `rnd/` is never read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::check::Finding;
use crate::screenshots::is_reference_screenshot;
use crate::tree;

/// TESTING.md, Fixtures and data files: 256 KiB.
const LIMIT_BYTES: u64 = 256 * 1024;

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
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if metadata.is_dir() {
            let relative = relative.to_string_lossy().replace('\\', "/");
            if tree::never_entered(&relative) {
                continue;
            }
            let fixture_root = name == tree::FIXTURES || relative == tree::PROOFS;
            walk(root, &path, in_fixtures || fixture_root, findings)?;
            continue;
        }
        if in_fixtures && metadata.len() > LIMIT_BYTES && !is_reference_screenshot(relative) {
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
