//! The pinned pair of TLA+ tools: `tla2tools.jar` and the `CommunityModules`
//! release's `CommunityModules-deps.jar`, each a URL with its SHA-256 in
//! `test-support/models/tools.toml` (ADR 0060). A kept file is used only
//! while its SHA-256 is its pin, so TLC never runs other tools than the pair.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::check::Finding;

/// The pinned tools, from the repository root.
pub(crate) const TOOLS_FILE: &str = "test-support/models/tools.toml";

/// Where `fetch` keeps the tools, from the repository root.
const KEPT_FOLDER: &str = "target/tla-tools";

/// The largest download kept; each tool is a few megabytes.
const MAXIMUM_DOWNLOAD: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsFile {
    tool: Vec<Tool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    /// The kept file's name, which TLC's class path names.
    name: String,
    url: String,
    /// Lowercase hexadecimal.
    sha256: String,
}

fn read(root: &Path) -> Result<Vec<Tool>> {
    let path = root.join(TOOLS_FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: ToolsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    for tool in &file.tool {
        let plain_name = !tool.name.is_empty()
            && !tool.name.starts_with('.')
            && !tool.name.contains(['/', '\\']);
        if !plain_name {
            bail!("{TOOLS_FILE}: `{}` is not a plain file name", tool.name);
        }
        let digest = tool.sha256.len() == 64
            && tool
                .sha256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if !digest {
            bail!(
                "{TOOLS_FILE}: {}'s sha256 is not 64 lowercase hexadecimal digits",
                tool.name
            );
        }
    }
    Ok(file.tool)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Whether the file at `path` exists with the SHA-256 `pin`.
fn kept_as_pinned(path: &Path, pin: &str) -> Result<bool> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(sha256(&bytes) == pin),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn download(url: &str) -> Result<Vec<u8>> {
    ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?
        .body_mut()
        .with_config()
        .limit(MAXIMUM_DOWNLOAD)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))
}

/// Downloads each tool whose kept file lacks its pinned SHA-256. A download
/// with another SHA-256 is a finding, and leaves no kept file of that tool.
pub(super) fn fetch(root: &Path) -> Result<Vec<Finding>> {
    let folder = root.join(KEPT_FOLDER);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let mut findings = Vec::new();
    for tool in read(root)? {
        let path = folder.join(&tool.name);
        if kept_as_pinned(&path, &tool.sha256)? {
            continue;
        }
        let bytes = download(&tool.url)?;
        let actual = sha256(&bytes);
        if actual != tool.sha256 {
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
            findings.push(Finding::new(
                "digest-mismatch",
                tool.name,
                format!(
                    "{} served SHA-256 {actual}, not the pinned {}; move the pin in {TOOLS_FILE} only after checking the new release",
                    tool.url, tool.sha256
                ),
            ));
            continue;
        }
        // Written beside its final name and renamed, so an interrupted
        // download never leaves a file under the kept name.
        let partial = folder.join(format!("{}.partial", tool.name));
        std::fs::write(&partial, &bytes)
            .with_context(|| format!("writing {}", partial.display()))?;
        std::fs::rename(&partial, &path)
            .with_context(|| format!("renaming {} to {}", partial.display(), path.display()))?;
    }
    Ok(findings)
}

/// The kept tools, in the pinned order of TLC's class path; fails unless
/// each has its pinned SHA-256.
pub(super) fn kept(root: &Path) -> Result<Vec<PathBuf>> {
    let folder = root.join(KEPT_FOLDER);
    read(root)?
        .into_iter()
        .map(|tool| {
            let path = folder.join(&tool.name);
            if !kept_as_pinned(&path, &tool.sha256)? {
                bail!(
                    "{} is missing or is not the pinned {}: run `cargo run -p xtask -- tlc fetch` before the deny",
                    path.display(),
                    tool.name
                );
            }
            Ok(path)
        })
        .collect()
}
