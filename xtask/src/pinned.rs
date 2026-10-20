//! Pinned downloads: files fetched before the outbound-traffic deny, each
//! named by a URL with its SHA-256 in a committed pins file, and used only
//! while the kept copy has that SHA-256 (ADR 0060's TLA+ tools and LazyFS
//! sources).
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

/// One pinned download.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    /// The kept file's name.
    pub(crate) name: String,
    pub(crate) url: String,
    /// Lowercase hexadecimal.
    pub(crate) sha256: String,
}

/// Reads the pins file at `relative` (from the repository root), whose
/// pins are the array `T` holds.
pub(crate) fn read<T: serde::de::DeserializeOwned>(
    root: &Path,
    relative: &str,
    pins: impl FnOnce(T) -> Vec<Pin>,
) -> Result<Vec<Pin>> {
    let path = root.join(relative);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: T = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let pins = pins(file);
    for pin in &pins {
        let plain_name =
            !pin.name.is_empty() && !pin.name.starts_with('.') && !pin.name.contains(['/', '\\']);
        if !plain_name {
            bail!("{relative}: `{}` is not a plain file name", pin.name);
        }
        let digest = pin.sha256.len() == 64
            && pin
                .sha256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if !digest {
            bail!(
                "{relative}: {}'s sha256 is not 64 lowercase hexadecimal digits",
                pin.name
            );
        }
    }
    Ok(pins)
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

fn download(url: &str, maximum: u64) -> Result<Vec<u8>> {
    ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?
        .body_mut()
        .with_config()
        .limit(maximum)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))
}

/// Downloads into `folder` each pin whose kept file lacks its SHA-256, each
/// at most `maximum` bytes. A download with another SHA-256 is a finding,
/// and leaves no kept file of that pin; `pins_file` names where to move it.
pub(crate) fn fetch(
    folder: &Path,
    pins: &[Pin],
    pins_file: &str,
    maximum: u64,
) -> Result<Vec<Finding>> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let mut findings = Vec::new();
    for pin in pins {
        let path = folder.join(&pin.name);
        if kept_as_pinned(&path, &pin.sha256)? {
            continue;
        }
        let bytes = download(&pin.url, maximum)?;
        let actual = sha256(&bytes);
        if actual != pin.sha256 {
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
            findings.push(Finding::new(
                "digest-mismatch",
                pin.name.clone(),
                format!(
                    "{} served SHA-256 {actual}, not the pinned {}; move the pin in {pins_file} only after checking the new release",
                    pin.url, pin.sha256
                ),
            ));
            continue;
        }
        // Written beside its final name and renamed, so an interrupted
        // download never leaves a file under the kept name.
        let partial = folder.join(format!("{}.partial", pin.name));
        std::fs::write(&partial, &bytes)
            .with_context(|| format!("writing {}", partial.display()))?;
        std::fs::rename(&partial, &path)
            .with_context(|| format!("renaming {} to {}", partial.display(), path.display()))?;
    }
    Ok(findings)
}

/// The kept files of `pins` in `folder`, in their pinned order: each one
/// missing or with another SHA-256 is a finding naming `fetch_command`.
pub(crate) fn kept(
    folder: &Path,
    pins: &[Pin],
    fetch_command: &str,
) -> Result<Result<Vec<PathBuf>, Vec<Finding>>> {
    let mut paths = Vec::new();
    let mut findings = Vec::new();
    for pin in pins {
        let path = folder.join(&pin.name);
        if kept_as_pinned(&path, &pin.sha256)? {
            paths.push(path);
        } else {
            findings.push(Finding::new(
                "not-pinned",
                pin.name.clone(),
                format!(
                    "{} is missing or is not the pinned {}: run `{fetch_command}` before the deny",
                    path.display(),
                    pin.name
                ),
            ));
        }
    }
    Ok(if findings.is_empty() {
        Ok(paths)
    } else {
        Err(findings)
    })
}
