//! `xtask release pin-clean-up-rules`: resolves and pins the newest passing
//! Clean-up rules daily set for a release line (ADRs 0030, 0032 and 0047;
//! issue 207).
//!
//! - On a release branch (`release/x.y`), pins the newest passing set of that
//!   release line from the OCI daily channel (never another line's, never a
//!   failing set, and never writing a pin file on `main`).
//! - On nightlies (`--line main`), resolves `main`'s newest passing set at
//!   build time without writing to `main`.
//! - Until the daily channel (#208) publishes, falls back to the committed
//!   seed set under `crates/goodfetch-clean-up-rules/rules/`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use ureq::Agent;

use crate::{git, output};

const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json";
const USER_AGENT: &str = "goodfetch-xtask (https://github.com/goodfetch/goodfetch)";
const SEED_DIR: &str = "crates/goodfetch-clean-up-rules/rules";
const FULL_ASSET: &str = "goodfetch-clean-up-rules-full.tar.xz";
const EMBEDDED_ASSET: &str = "goodfetch-clean-up-rules-embedded.txt";

const ANN_LINE: &str = "dev.goodfetch.clean-up-rules.line";
const ANN_VERSION: &str = "dev.goodfetch.clean-up-rules.version";
const ANN_GATE: &str = "dev.goodfetch.clean-up-rules.gate";
const ANN_PART: &str = "dev.goodfetch.clean-up-rules.part";

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Release line to pin (`main`, `release/1.0`, or `1.0`).
    #[arg(long)]
    line: String,
    /// Optional OCI Distribution registry base URL (`https://ghcr.io` or local `registry:2`).
    #[arg(long)]
    registry: Option<String>,
    /// OCI repository holding daily Clean-up rules sets.
    #[arg(long, default_value = "goodfetch/clean-up-rules")]
    repository: String,
    /// Optional pin file written on a release branch; refused on `main`.
    #[arg(long)]
    pin_file: Option<PathBuf>,
    /// Optional assets folder where the full set and embedded rule text are
    /// written.
    #[arg(long)]
    out_assets: Option<PathBuf>,
    /// Optional path where the Markdown release-notes snippet (version +
    /// SHA-256 digests) is written (kept separate from `--out-assets` so notes
    /// are never uploaded or attested as a release asset).
    #[arg(long)]
    out_notes: Option<PathBuf>,
}

#[derive(Deserialize)]
struct TagsList {
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OciManifest {
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    #[serde(default)]
    layers: Vec<OciLayer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OciLayer {
    digest: String,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct SeedManifest {
    published_at: String,
    lists: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct PinRecord {
    line: String,
    version: String,
    source: String,
    full_asset: String,
    full_sha256: String,
    embedded_asset: String,
    embedded_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    oci_tag: Option<String>,
}

struct ResolvedSet {
    line: String,
    version: String,
    source: String,
    oci_tag: Option<String>,
    full_bytes: Vec<u8>,
    embedded_bytes: Vec<u8>,
}

/// Default committed pin path on a release branch (never written on `main`).
pub(super) const DEFAULT_PIN_FILE: &str = "crates/goodfetch-clean-up-rules/pin.json";
const DEFAULT_REPOSITORY: &str = "goodfetch/clean-up-rules";

/// Pins `line`'s newest passing Clean-up rules daily set from `registry` into
/// `DEFAULT_PIN_FILE` (refusing if `line == "main"` or `HEAD` is `main`).
pub(super) fn pin_release_line(
    root: &Path,
    line: &str,
    registry: &str,
    token: Option<&str>,
) -> Result<()> {
    let args = Arguments {
        line: line.to_owned(),
        registry: Some(registry.to_owned()),
        repository: DEFAULT_REPOSITORY.to_owned(),
        pin_file: Some(PathBuf::from(DEFAULT_PIN_FILE)),
        out_assets: None,
        out_notes: None,
    };
    let _ = run(root, &args, token)?;
    Ok(())
}

pub(super) fn run(root: &Path, arguments: &Arguments, token: Option<&str>) -> Result<ExitCode> {
    let line = normalize_line(&arguments.line);
    ensure!(!line.is_empty(), "--line must not be empty");

    if arguments.pin_file.is_some() {
        ensure!(
            line != "main",
            "refusing to write a Clean-up rules pin file for `main`: nightlies pin at build time without committing to `main` (ADR 0047)"
        );
        if let Ok(branch) = git::line(root, &["symbolic-ref", "--short", "HEAD"]) {
            ensure!(
                branch != "main",
                "refusing to write a Clean-up rules pin file while checked out on `main` (ADR 0047)"
            );
        }
    }

    let resolved = match &arguments.registry {
        Some(registry) => {
            if let Some(from_oci) =
                resolve_from_registry(registry, &arguments.repository, &line, token)?
            {
                from_oci
            } else {
                resolve_from_seed(root, &line)?
            }
        }
        None => resolve_from_seed(root, &line)?,
    };

    let full_sha = format!("{:x}", Sha256::digest(&resolved.full_bytes));
    let embedded_sha = format!("{:x}", Sha256::digest(&resolved.embedded_bytes));

    if let Some(out_assets) = &arguments.out_assets {
        let dir = root.join(out_assets);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        std::fs::write(dir.join(FULL_ASSET), &resolved.full_bytes)?;
        std::fs::write(dir.join(EMBEDDED_ASSET), &resolved.embedded_bytes)?;
    }

    if let Some(out_notes) = &arguments.out_notes {
        let notes_path = root.join(out_notes);
        if let Some(parent) = notes_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let notes = format!(
            "## Clean-up rules\n\n\
             - Version: `{}` (line `{}`, source `{}`)\n\
             - Full set (`{FULL_ASSET}`): `sha256:{full_sha}`\n\
             - Embedded rule text (`{EMBEDDED_ASSET}`): `sha256:{embedded_sha}`\n",
            resolved.version, resolved.line, resolved.source
        );
        std::fs::write(&notes_path, notes)
            .with_context(|| format!("writing {}", notes_path.display()))?;
    }

    if let Some(pin_file) = &arguments.pin_file {
        let path = root.join(pin_file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let record = PinRecord {
            line: resolved.line.clone(),
            version: resolved.version.clone(),
            source: resolved.source.clone(),
            full_asset: FULL_ASSET.to_owned(),
            full_sha256: full_sha.clone(),
            embedded_asset: EMBEDDED_ASSET.to_owned(),
            embedded_sha256: embedded_sha.clone(),
            oci_tag: resolved.oci_tag.clone(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&record)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }

    let summary = json!({
        "line": resolved.line,
        "version": resolved.version,
        "source": resolved.source,
        "full_sha256": full_sha,
        "embedded_sha256": embedded_sha,
    });
    output::line(&summary.to_string());
    Ok(ExitCode::SUCCESS)
}

fn normalize_line(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("release/")
        .unwrap_or(trimmed)
        .to_owned()
}

fn version_sort_key(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

struct OciPullAuth<'a> {
    agent: &'a Agent,
    repository: &'a str,
    github_token: Option<&'a str>,
    bearer: Option<String>,
}

impl OciPullAuth<'_> {
    fn get(&mut self, url: &str, accept: Option<&str>) -> Result<ureq::http::Response<ureq::Body>> {
        let send = |bearer: Option<&str>| {
            let mut req = with_auth(self.agent.get(url), bearer);
            if let Some(accept) = accept {
                req = req.header("Accept", accept);
            }
            req.call()
        };
        let mut resp = send(self.bearer.as_deref().or(self.github_token))?;
        if resp.status() == 401
            && let Some(header) = resp
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok())
        {
            let scope = format!("repository:{}:pull", self.repository);
            let exchanged = super::publish::oci::exchange_bearer_token(
                self.agent,
                header,
                self.repository,
                &scope,
                self.github_token,
            )?;
            self.bearer = Some(exchanged);
            resp = send(self.bearer.as_deref())?;
        }
        Ok(resp)
    }
}

fn resolve_from_registry(
    registry: &str,
    repository: &str,
    line: &str,
    token: Option<&str>,
) -> Result<Option<ResolvedSet>> {
    let base = registry.trim_end_matches('/');
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into();
    let mut auth = OciPullAuth {
        agent: &agent,
        repository,
        github_token: token,
        bearer: None,
    };

    let mut tags = Vec::new();
    let mut next_url = Some(format!("{base}/v2/{repository}/tags/list"));
    while let Some(url) = next_url {
        let mut resp = auth.get(&url, None)?;
        if resp.status() == 404 {
            return Ok(None);
        }
        if resp.status() != 200 {
            bail!("{url} answered {}", resp.status());
        }
        next_url = resp
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .and_then(super::publish::oci::next_page)
            .map(|next| {
                if next.starts_with("http://") || next.starts_with("https://") {
                    next.to_owned()
                } else {
                    format!("{base}{next}")
                }
            });
        let list: TagsList = serde_json::from_str(&resp.body_mut().read_to_string()?)
            .with_context(|| format!("parsing tags list from {url}"))?;
        tags.extend(list.tags.unwrap_or_default());
    }
    if tags.is_empty() {
        return Ok(None);
    }

    let mut best: Option<(Vec<u64>, String, String, OciManifest)> = None;
    for tag in tags {
        let manifest_url = format!("{base}/v2/{repository}/manifests/{tag}");
        let mut mresp = auth.get(&manifest_url, Some(MANIFEST_ACCEPT))?;
        if mresp.status() != 200 {
            continue;
        }
        let manifest: OciManifest = serde_json::from_str(&mresp.body_mut().read_to_string()?)
            .with_context(|| format!("parsing manifest for {tag}"))?;
        let m_line = manifest
            .annotations
            .get(ANN_LINE)
            .map(|s| normalize_line(s));
        let m_gate = manifest.annotations.get(ANN_GATE).map(String::as_str);
        let Some(m_version) = manifest.annotations.get(ANN_VERSION).cloned() else {
            continue;
        };
        if m_line.as_deref() != Some(line) || m_gate != Some("passed") {
            continue;
        }
        let key = version_sort_key(&m_version);
        let replace = match &best {
            None => true,
            Some((best_key, best_ver, _, _)) => {
                key > *best_key || (key == *best_key && m_version > *best_ver)
            }
        };
        if replace {
            best = Some((key, m_version, tag, manifest));
        }
    }

    let Some((_, version, tag, manifest)) = best else {
        bail!("no passing Clean-up rules daily set found in {base}/{repository} for line `{line}`");
    };

    let full_layer = manifest
        .layers
        .iter()
        .find(|layer| layer.annotations.get(ANN_PART).map(String::as_str) == Some("full"))
        .with_context(|| format!("manifest {tag} missing `full` layer"))?;
    let embedded_layer = manifest
        .layers
        .iter()
        .find(|layer| layer.annotations.get(ANN_PART).map(String::as_str) == Some("embedded"))
        .with_context(|| format!("manifest {tag} missing `embedded` layer"))?;

    let full_bytes = fetch_verified_blob(&mut auth, base, repository, &full_layer.digest)?;
    let embedded_bytes = fetch_verified_blob(&mut auth, base, repository, &embedded_layer.digest)?;

    Ok(Some(ResolvedSet {
        line: line.to_owned(),
        version,
        source: format!("{base}/{repository}:{tag}"),
        oci_tag: Some(tag),
        full_bytes,
        embedded_bytes,
    }))
}

fn fetch_verified_blob(
    auth: &mut OciPullAuth<'_>,
    base: &str,
    repository: &str,
    digest: &str,
) -> Result<Vec<u8>> {
    let url = format!("{base}/v2/{repository}/blobs/{digest}");
    let mut resp = auth.get(&url, None)?;
    if resp.status() != 200 {
        bail!("{url} answered {}", resp.status());
    }
    let bytes = resp.body_mut().read_to_vec()?;
    let expected = digest
        .strip_prefix("sha256:")
        .with_context(|| format!("unsupported digest format {digest}"))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    ensure!(
        actual == expected,
        "blob {digest} digest mismatch: expected {expected}, got {actual}"
    );
    Ok(bytes)
}

fn resolve_from_seed(root: &Path, line: &str) -> Result<ResolvedSet> {
    let seed_dir = root.join(SEED_DIR);
    let manifest_path = seed_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading seed Clean-up rules {}", manifest_path.display()))?;
    let manifest: SeedManifest = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    // Concatenate the committed embedded lists in deterministic key order.
    let mut embedded = String::new();
    for (category, file) in &manifest.lists {
        let path = seed_dir.join(file);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading seed list {}", path.display()))?;
        let _ = writeln!(
            embedded,
            "! goodfetch-clean-up-rules category={category} file={file}"
        );
        embedded.push_str(&text);
        if !embedded.ends_with('\n') {
            embedded.push('\n');
        }
    }

    // Build an XZ-compressed deterministic `.tar.xz` archive of the committed seed set.
    let staging = tempfile::tempdir().context("creating Clean-up rules seed staging folder")?;
    let embedded_path = staging.path().join(EMBEDDED_ASSET);
    std::fs::write(&embedded_path, embedded.as_bytes())
        .with_context(|| format!("writing {}", embedded_path.display()))?;
    let archive_path = staging.path().join(FULL_ASSET);
    let status = std::process::Command::new("tar")
        .args(["-cJf"])
        .arg(&archive_path)
        .args(["-C"])
        .arg(staging.path())
        .arg(EMBEDDED_ASSET)
        .status()
        .with_context(|| format!("running `tar -cJf {}`", archive_path.display()))?;
    anyhow::ensure!(
        status.success(),
        "`tar -cJf {}` exited with {status}",
        archive_path.display()
    );
    let full_bytes = std::fs::read(&archive_path)
        .with_context(|| format!("reading {}", archive_path.display()))?;

    Ok(ResolvedSet {
        line: line.to_owned(),
        version: format!("seed-{}", manifest.published_at),
        source: "committed-seed".to_owned(),
        oci_tag: None,
        full_bytes,
        embedded_bytes: embedded.into_bytes(),
    })
}

fn with_auth<T>(request: ureq::RequestBuilder<T>, token: Option<&str>) -> ureq::RequestBuilder<T> {
    match token {
        Some(token) if !token.is_empty() => {
            request.header("Authorization", &format!("Bearer {token}"))
        }
        _ => request,
    }
}
