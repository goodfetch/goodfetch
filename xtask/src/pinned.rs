//! Pinned downloads: files fetched before the outbound-traffic deny, each
//! named by its source, a URL or a GitHub release's asset, with its SHA-256
//! in a committed pins file, and used only while the kept copy has that
//! SHA-256 (ADR 0060's TLA+ tools and LazyFS sources).
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
use crate::github::Api;

/// One pinned download.
#[derive(Deserialize)]
#[serde(try_from = "PinEntry")]
pub(crate) struct Pin {
    /// The kept file's name.
    pub(crate) name: String,
    pub(crate) source: Source,
    /// Lowercase hexadecimal.
    pub(crate) sha256: String,
}

/// Where a pinned file is downloaded from.
pub(crate) enum Source {
    /// A URL anyone may read.
    Url(String),
    /// An asset of a GitHub release, read through GitHub's API with the
    /// job's token, so a private repository's release serves it too.
    Release(ReleaseAsset),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseAsset {
    /// The repository, as `owner/name`.
    pub(crate) repository: String,
    pub(crate) tag: String,
    /// The asset's name in the release.
    pub(crate) asset: String,
}

/// A pin as its file writes it: exactly one of `url` and `release`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinEntry {
    name: String,
    url: Option<String>,
    release: Option<ReleaseAsset>,
    sha256: String,
}

impl TryFrom<PinEntry> for Pin {
    type Error = String;

    fn try_from(entry: PinEntry) -> Result<Self, Self::Error> {
        let source = match (entry.url, entry.release) {
            (Some(url), None) => Source::Url(url),
            (None, Some(release)) => Source::Release(release),
            _ => {
                return Err(format!(
                    "{} needs exactly one of `url` and `release`",
                    entry.name
                ));
            }
        };
        Ok(Self {
            name: entry.name,
            source,
            sha256: entry.sha256,
        })
    }
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

/// A release as GitHub's API lists it, with only what a download needs.
#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
}

/// What a pin's source served.
enum Downloaded<'a> {
    Bytes(Vec<u8>),
    /// The release does not exist, or the token cannot see it.
    NoRelease(&'a ReleaseAsset),
    /// The release exists without the pinned asset.
    NoAsset,
}

fn download_source<'a>(source: &'a Source, api: Api<'_>, maximum: u64) -> Result<Downloaded<'a>> {
    match source {
        Source::Url(url) => download(url, maximum).map(Downloaded::Bytes),
        Source::Release(release) => {
            let Some(listing) = api.find::<Release>(&format!(
                "/repos/{}/releases/tags/{}",
                release.repository, release.tag
            ))?
            else {
                return Ok(Downloaded::NoRelease(release));
            };
            let Some(asset) = listing
                .assets
                .into_iter()
                .find(|asset| asset.name == release.asset)
            else {
                return Ok(Downloaded::NoAsset);
            };
            api.download(
                &format!("/repos/{}/releases/assets/{}", release.repository, asset.id),
                maximum,
            )
            .map(Downloaded::Bytes)
        }
    }
}

impl Source {
    /// Where the pin comes from, for a finding.
    fn describe(&self) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::Release(release) => format!(
                "{} of {}'s release {}",
                release.asset, release.repository, release.tag
            ),
        }
    }
}

/// Downloads into `folder` each pin whose kept file lacks its SHA-256, each
/// at most `maximum` bytes, a release's asset through `api`. A download with
/// another SHA-256, a missing release and a release without the pinned asset
/// are each a finding, and leave no kept file of that pin; `pins_file` names
/// where to correct it.
pub(crate) fn fetch(
    folder: &Path,
    pins: &[Pin],
    pins_file: &str,
    maximum: u64,
    api: Api<'_>,
) -> Result<Vec<Finding>> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let mut findings = Vec::new();
    for pin in pins {
        let path = folder.join(&pin.name);
        if kept_as_pinned(&path, &pin.sha256)? {
            continue;
        }
        let finding = match download_source(&pin.source, api, maximum)? {
            Downloaded::Bytes(bytes) if sha256(&bytes) == pin.sha256 => {
                keep(folder, &pin.name, &bytes)?;
                continue;
            }
            Downloaded::Bytes(bytes) => Finding::new(
                "digest-mismatch",
                pin.name.clone(),
                format!(
                    "{} served SHA-256 {}, not the pinned {}; move the pin in {pins_file} only after checking the new release",
                    pin.source.describe(),
                    sha256(&bytes),
                    pin.sha256
                ),
            ),
            Downloaded::NoRelease(release) => Finding::new(
                "missing-release",
                pin.name.clone(),
                format!(
                    "{}'s release {} does not exist, or GITHUB_TOKEN cannot read it: create it with the asset {} that {pins_file} pins, or set the token",
                    release.repository, release.tag, release.asset
                ),
            ),
            Downloaded::NoAsset => Finding::new(
                "missing-asset",
                pin.name.clone(),
                format!(
                    "{} has no asset of that name: attach it, or correct the pin in {pins_file}",
                    pin.source.describe()
                ),
            ),
        };
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
        findings.push(finding);
    }
    Ok(findings)
}

/// Writes `bytes` as the kept file `name` in `folder`, beside its final name
/// first and then renamed, so an interrupted download never leaves a file
/// under the kept name.
fn keep(folder: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let path = folder.join(name);
    let partial = folder.join(format!("{name}.partial"));
    std::fs::write(&partial, bytes).with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, &path)
        .with_context(|| format!("renaming {} to {}", partial.display(), path.display()))
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
