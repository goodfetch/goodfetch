//! The pinned pair of TLA+ tools: `tla2tools.jar` and the `CommunityModules`
//! release's `CommunityModules-deps.jar`, each a URL with its SHA-256 in
//! `test-support/models/tools.toml` (ADR 0060). A kept file is used only
//! while its SHA-256 is its pin, so TLC never runs other tools than the pair.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::check::Finding;
use crate::pinned::{self, Pin};

/// The pinned tools, from the repository root.
pub(crate) const TOOLS_FILE: &str = "test-support/models/tools.toml";

/// Where `fetch` keeps the tools, from the repository root.
const KEPT_FOLDER: &str = "target/tla-tools";

/// The largest download kept; each tool is a few megabytes.
const MAXIMUM_DOWNLOAD: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsFile {
    tool: Vec<Pin>,
}

fn read(root: &Path) -> Result<Vec<Pin>> {
    pinned::read(root, TOOLS_FILE, |file: ToolsFile| file.tool)
}

/// Downloads each tool whose kept file lacks its pinned SHA-256. A download
/// with another SHA-256 is a finding, and leaves no kept file of that tool.
pub(super) fn fetch(root: &Path) -> Result<Vec<Finding>> {
    pinned::fetch(
        &root.join(KEPT_FOLDER),
        &read(root)?,
        TOOLS_FILE,
        MAXIMUM_DOWNLOAD,
    )
}

/// The kept tools, in the pinned order of TLC's class path; fails unless
/// each has its pinned SHA-256.
pub(super) fn kept(root: &Path) -> Result<Vec<PathBuf>> {
    match pinned::kept(
        &root.join(KEPT_FOLDER),
        &read(root)?,
        "cargo run -p xtask -- tlc fetch",
    )? {
        Ok(paths) => Ok(paths),
        Err(findings) => bail!(
            "{}",
            findings
                .iter()
                .map(|finding| finding.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}
