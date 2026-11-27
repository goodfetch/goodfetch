//! `xtask release fetch-chromium` and `chromium-tree-hashes`: the release
//! pipeline never builds the Goodfetch Chromium (ADR 0074). It verifies the
//! committed manifest's input tree hashes against this commit, checks each
//! artifact's build and verdict attestations, then fetches the bundles and
//! chromedriver by SHA-256 digest.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::check::{self, Finding};
use crate::output;

/// Default path of the committed Goodfetch Chromium digest manifest (ADR 0074).
const DEFAULT_MANIFEST: &str = "chromium/manifest.toml";
/// Default workflow file for the Chromium build gate, part of the test inputs.
const GATE_WORKFLOW: &str = ".github/workflows/chromium-gate.yml";
/// Files under `chromium/` that are outputs of the gate, never build inputs.
const OUTPUT_FILES: [&str; 2] = ["chromium/manifest.toml", "chromium/protocol.json"];
const EXPECTATIONS_PREFIX: &str = "chromium/expectations/";

#[derive(clap::Args)]
pub(crate) struct FetchArguments {
    /// Path to the committed Goodfetch Chromium manifest.
    #[arg(long, default_value = DEFAULT_MANIFEST)]
    manifest: PathBuf,
    /// Folder where verified bundles and chromedriver archives are saved.
    #[arg(long)]
    out: PathBuf,
    /// Local attestation folder (`<sha256>.build.json` and `<sha256>.verdict.json`)
    /// for offline verification; when omitted, `gh attestation verify` is used.
    #[arg(long)]
    attestations: Option<PathBuf>,
    /// Repository owner/name for `gh attestation verify`.
    #[arg(long, default_value = "goodfetch/goodfetch")]
    repository: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    workflow: String,
    build_input_tree: String,
    test_input_tree: String,
    #[serde(default)]
    artifact: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    #[expect(
        dead_code,
        reason = "deserialized from manifest for schema completeness"
    )]
    target: String,
    #[expect(
        dead_code,
        reason = "deserialized from manifest for schema completeness"
    )]
    kind: String,
    filename: String,
    url: String,
    sha256: String,
}

#[derive(Serialize)]
struct TreeHashes {
    build_input_tree: String,
    test_input_tree: String,
}

#[derive(Deserialize)]
struct BuildAttestation {
    subject_sha256: String,
    workflow: String,
    build_input_tree: String,
}

#[derive(Deserialize)]
struct VerdictAttestation {
    subject_sha256: String,
    workflow: String,
    build_input_tree: String,
    test_input_tree: String,
    verdict: String,
}

/// Prints the workspace's Goodfetch Chromium build-input and test-input tree
/// hashes as JSON.
pub(super) fn tree_hashes(root: &Path) -> Result<ExitCode> {
    let hashes = compute_tree_hashes(root)?;
    output::line(&serde_json::to_string_pretty(&hashes)?);
    Ok(ExitCode::SUCCESS)
}

/// Verifies the committed Goodfetch Chromium manifest against this commit's
/// input tree hashes and each digest's build and verdict attestations, then
/// fetches the verified files into `--out`. Never builds Chromium.
pub(super) fn fetch(root: &Path, arguments: &FetchArguments) -> Result<ExitCode> {
    let manifest_path = root.join(&arguments.manifest);
    if !manifest_path.is_file() {
        output::line(
            "no Goodfetch Chromium manifest is committed yet: nothing fetched, nothing built",
        );
        return Ok(ExitCode::SUCCESS);
    }

    let manifest_rel = arguments.manifest.display().to_string();
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: Manifest =
        toml::from_str(&text).with_context(|| format!("parsing {}", manifest_path.display()))?;

    if manifest.artifact.is_empty() {
        output::line(
            "no Goodfetch Chromium artifacts are listed in manifest: nothing fetched, nothing built",
        );
        return Ok(ExitCode::SUCCESS);
    }

    let actual = compute_tree_hashes(root)?;
    if manifest.build_input_tree != actual.build_input_tree
        || manifest.test_input_tree != actual.test_input_tree
    {
        return Ok(check::report(&[Finding::new(
            "input-tree-mismatch",
            &manifest_rel,
            format!(
                "manifest tree hashes (build={}, test={}) do not match this commit's inputs (build={}, test={})",
                manifest.build_input_tree,
                manifest.test_input_tree,
                actual.build_input_tree,
                actual.test_input_tree
            ),
        )]));
    }

    let mut findings = Vec::new();
    for artifact in &manifest.artifact {
        if let Err(reason) = verify_attestations(
            root,
            &manifest,
            artifact,
            arguments.attestations.as_deref(),
            &arguments.repository,
        ) {
            findings.push(Finding::new(
                "attestation-invalid",
                &artifact.filename,
                reason,
            ));
        }
    }
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }

    let out_dir = root.join(&arguments.out);
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    for artifact in &manifest.artifact {
        let dest = out_dir.join(&artifact.filename);
        let downloaded = fetch_and_verify(&artifact.url, &dest, &artifact.sha256)?;
        if let Err(actual_digest) = downloaded {
            findings.push(Finding::new(
                "digest-mismatch",
                &artifact.filename,
                format!(
                    "expected sha256 {} from manifest, fetched {}",
                    artifact.sha256, actual_digest
                ),
            ));
        } else {
            output::line(&dest.display().to_string());
        }
    }

    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }
    Ok(ExitCode::SUCCESS)
}

/// Computes deterministic SHA-256 tree hashes of the Chromium build inputs
/// and test inputs (ADR 0074).
fn compute_tree_hashes(root: &Path) -> Result<TreeHashes> {
    let mut build_files = BTreeMap::new();
    let mut test_files = BTreeMap::new();

    let chromium_dir = root.join("chromium");
    if chromium_dir.is_dir() {
        collect_files(root, &chromium_dir, &mut |rel, path| {
            if OUTPUT_FILES.contains(&rel.as_str()) {
                return Ok(());
            }
            if rel.starts_with(EXPECTATIONS_PREFIX) {
                test_files.insert(rel, path);
            } else {
                build_files.insert(rel, path);
            }
            Ok(())
        })?;
    }

    let gate_workflow = root.join(GATE_WORKFLOW);
    if gate_workflow.is_file() {
        test_files.insert(GATE_WORKFLOW.to_owned(), gate_workflow);
    }

    Ok(TreeHashes {
        build_input_tree: hash_tree(&build_files)?,
        test_input_tree: hash_tree(&test_files)?,
    })
}

fn collect_files(
    root: &Path,
    dir: &Path,
    visitor: &mut impl FnMut(String, PathBuf) -> Result<()>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(root, &path, visitor)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("stripping {} from {}", root.display(), path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            visitor(rel, path)?;
        }
    }
    Ok(())
}

fn hash_tree(files: &BTreeMap<String, PathBuf>) -> Result<String> {
    let mut hasher = Sha256::new();
    for (rel, path) in files {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update([0]);
        hasher.update(&bytes);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_attestations(
    root: &Path,
    manifest: &Manifest,
    artifact: &Artifact,
    local_dir: Option<&Path>,
    repository: &str,
) -> std::result::Result<(), String> {
    if let Some(dir) = local_dir {
        let base = root.join(dir);
        let build_path = base.join(format!("{}.build.json", artifact.sha256));
        let build_text = std::fs::read_to_string(&build_path)
            .map_err(|_| format!("missing build attestation {}", build_path.display()))?;
        let build: BuildAttestation = serde_json::from_str(&build_text)
            .map_err(|error| format!("invalid build attestation: {error}"))?;
        if build.subject_sha256 != artifact.sha256
            || build.workflow != manifest.workflow
            || build.build_input_tree != manifest.build_input_tree
        {
            return Err(
                "build attestation does not match digest, workflow, or build input tree".to_owned(),
            );
        }

        let verdict_path = base.join(format!("{}.verdict.json", artifact.sha256));
        let verdict_text = std::fs::read_to_string(&verdict_path)
            .map_err(|_| format!("missing verdict attestation {}", verdict_path.display()))?;
        let verdict: VerdictAttestation = serde_json::from_str(&verdict_text)
            .map_err(|error| format!("invalid verdict attestation: {error}"))?;
        if verdict.subject_sha256 != artifact.sha256
            || verdict.workflow != manifest.workflow
            || verdict.build_input_tree != manifest.build_input_tree
            || verdict.test_input_tree != manifest.test_input_tree
            || verdict.verdict != "pass"
        {
            return Err(
                "verdict attestation is not a passing verdict for this commit's input trees"
                    .to_owned(),
            );
        }
        return Ok(());
    }

    // Live CI path: verify via `gh attestation verify` against the Chromium build gate workflow.
    let status = Command::new("gh")
        .args([
            "attestation",
            "verify",
            "--repo",
            repository,
            "--signer-workflow",
            &format!("{repository}/{}", manifest.workflow),
            "--digest",
            &format!("sha256:{}", artifact.sha256),
        ])
        .output()
        .map_err(|error| format!("running `gh attestation verify`: {error}"))?;
    if !status.status.success() {
        return Err(format!(
            "`gh attestation verify` failed: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    Ok(())
}

/// Downloads `url` to a temporary file beside `dest`, checks its SHA-256
/// against `expected`, and renames it into `dest` only on an exact match.
fn fetch_and_verify(
    url: &str,
    dest: &Path,
    expected: &str,
) -> Result<std::result::Result<(), String>> {
    ensure_url(url)?;
    let partial = dest.with_extension("partial");
    let mut sink =
        File::create(&partial).with_context(|| format!("creating {}", partial.display()))?;
    let mut hasher = Sha256::new();

    if let Some(path) = url.strip_prefix("file://") {
        let mut source = File::open(path).with_context(|| format!("opening {path}"))?;
        copy_and_hash(&mut source, &mut sink, &mut hasher)?;
    } else {
        let response = ureq::get(url)
            .call()
            .with_context(|| format!("fetching {url}"))?;
        let mut reader = response.into_body().into_reader();
        copy_and_hash(&mut reader, &mut sink, &mut hasher)?;
    }
    sink.flush()?;
    drop(sink);

    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        let _ = std::fs::remove_file(&partial);
        return Ok(Err(actual));
    }
    std::fs::rename(&partial, dest).with_context(|| {
        format!(
            "moving verified artifact {} into {}",
            partial.display(),
            dest.display()
        )
    })?;
    Ok(Ok(()))
}

fn copy_and_hash(
    reader: &mut impl io::Read,
    writer: &mut impl io::Write,
    hasher: &mut Sha256,
) -> Result<()> {
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .context("reading artifact stream")?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .context("writing artifact")?;
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

fn ensure_url(url: &str) -> Result<()> {
    if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("file://") {
        Ok(())
    } else {
        bail!("unsupported artifact URL scheme: {url}")
    }
}
