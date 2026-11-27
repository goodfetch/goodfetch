//! `xtask chromium gate` (ADR 0074, ticket #400): computes the two Git tree
//! hashes (build inputs vs test inputs, excluding outputs: the digest
//! manifest, the protocol JSON and spec #600's per-platform HTTP profiles),
//! checks build and verdict attestations plus committed HTTP profiles for the
//! fast path, fails on mismatches in the merge queue, and writes JUnit XML
//! under the `chromium-build` tier.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::git;
use crate::output;

/// The committed digest manifest of published Goodfetch Chromium builds.
const MANIFEST: &str = "chromium/manifest.json";

/// Outputs excluded from both input tree hashes (ADR 0074 + spec #600).
const PROTOCOL_OUTPUT: &str = "chromium/protocol.json";
const HTTP_PROFILES_DIR: &str = "chromium/http-profiles/";

/// The gate's own workflow file, part of the test inputs so changing how the
/// gate runs never passes on an old verdict.
const GATE_WORKFLOW: &str = ".github/workflows/chromium-build-gate.yml";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints `build_inputs=<git-tree>` and `test_inputs=<git-tree>`.
    Hashes,
    /// Runs the Chromium build gate's fast-path and attestation check.
    Check {
        /// Triggering GitHub event (`pull_request` or `merge_group`).
        #[arg(long, default_value = "pull_request")]
        event: String,
        /// Optional JSON file of build and verdict attestations (Seam A).
        #[arg(long)]
        attestations: Option<PathBuf>,
        /// Optional path to write a JUnit XML report for the `chromium-build` tier.
        #[arg(long)]
        junit: Option<PathBuf>,
    },
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DigestManifest {
    #[serde(default)]
    builds: Vec<PublishedBuild>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedBuild {
    platform: String,
    digest: String,
    #[serde(default)]
    http_profile: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttestationsFile {
    #[serde(default)]
    builds: Vec<BuildAttestation>,
    #[serde(default)]
    verdicts: Vec<VerdictAttestation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildAttestation {
    digest: String,
    build_inputs: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerdictAttestation {
    digest: String,
    build_inputs: String,
    test_inputs: String,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let root = &environment.root;
    match command {
        Command::Hashes => {
            let (build_hash, test_hash) = input_tree_hashes(root)?;
            output::line(&format!("build_inputs={build_hash}"));
            output::line(&format!("test_inputs={test_hash}"));
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            event,
            attestations,
            junit,
        } => {
            let (build_hash, test_hash) = input_tree_hashes(root)?;
            output::line(&format!("build_inputs={build_hash}"));
            output::line(&format!("test_inputs={test_hash}"));
            let manifest = read_manifest(&root.join(MANIFEST))?;
            if manifest.builds.is_empty() {
                output::line("verdict=pass");
                output::line("fast_path=empty-manifest");
                if let Some(junit_path) = junit {
                    write_junit(
                        &environment.path(junit_path),
                        "fast-path-empty-manifest",
                        None,
                    )?;
                }
                return Ok(ExitCode::SUCCESS);
            }
            let attestations = match attestations {
                Some(path) => read_attestations(&environment.path(path))?,
                None => AttestationsFile::default(),
            };
            let findings = verify_builds(
                root,
                &manifest.builds,
                &attestations,
                &build_hash,
                &test_hash,
                event,
            );
            if findings.is_empty() {
                output::line("verdict=pass");
                output::line("fast_path=hit");
                if let Some(junit_path) = junit {
                    write_junit(&environment.path(junit_path), "fast-path", None)?;
                }
                Ok(ExitCode::SUCCESS)
            } else {
                output::line("verdict=fail");
                output::line("fast_path=miss");
                let summary = findings
                    .iter()
                    .map(|f| format!("{} ({}): {}", f.kind, f.subject, f.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                if let Some(junit_path) = junit {
                    write_junit(&environment.path(junit_path), "fast-path", Some(&summary))?;
                }
                Ok(check::report(&findings))
            }
        }
    }
}

fn verify_builds(
    root: &Path,
    builds: &[PublishedBuild],
    attestations: &AttestationsFile,
    build_hash: &str,
    test_hash: &str,
    event: &str,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for build in builds {
        let profile_rel = build
            .http_profile
            .clone()
            .unwrap_or_else(|| format!("{HTTP_PROFILES_DIR}{}.json", build.platform));
        if !root.join(&profile_rel).is_file() {
            findings.push(Finding::new(
                "missing-http-profile",
                &build.platform,
                format!(
                    "published digest `{}` has no committed HTTP profile at `{profile_rel}` (spec #600)",
                    build.digest
                ),
            ));
        }
        let build_ok = attestations
            .builds
            .iter()
            .any(|att| att.digest == build.digest && att.build_inputs == build_hash);
        if !build_ok {
            findings.push(Finding::new(
                "build-attestation-mismatch",
                &build.platform,
                format!(
                    "digest `{}` lacks a build attestation matching build_inputs `{build_hash}`",
                    build.digest
                ),
            ));
        }
        let verdict_ok = attestations.verdicts.iter().any(|att| {
            att.digest == build.digest
                && att.build_inputs == build_hash
                && att.test_inputs == test_hash
        });
        if !verdict_ok {
            let suffix = if event == "merge_group" {
                " (merge queue fails on a mismatch; run the gate in the pull request)"
            } else {
                ""
            };
            findings.push(Finding::new(
                "verdict-attestation-mismatch",
                &build.platform,
                format!(
                    "digest `{}` lacks a verdict attestation matching build_inputs `{build_hash}` and test_inputs `{test_hash}`{suffix}",
                    build.digest
                ),
            ));
        }
    }
    findings
}

fn read_manifest(path: &Path) -> Result<DigestManifest> {
    if !path.is_file() {
        return Ok(DigestManifest::default());
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn read_attestations(path: &Path) -> Result<AttestationsFile> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Computes the two genuine Git tree hashes for build inputs and test inputs.
fn input_tree_hashes(root: &Path) -> Result<(String, String)> {
    let mut build_files = BTreeMap::new();
    let mut test_files = BTreeMap::new();

    let chromium_dir = root.join("chromium");
    if chromium_dir.is_dir() {
        collect_chromium_files(&chromium_dir, "chromium", &mut build_files, &mut test_files)?;
    }
    let workflow_path = root.join(GATE_WORKFLOW);
    if workflow_path.is_file() {
        test_files.insert(GATE_WORKFLOW.to_owned(), workflow_path);
    }

    let build_hash = git_tree_hash(&build_files)?;
    let test_hash = git_tree_hash(&test_files)?;
    Ok((build_hash, test_hash))
}

fn collect_chromium_files(
    dir: &Path,
    rel_prefix: &str,
    build_files: &mut BTreeMap<String, PathBuf>,
    test_files: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("non-UTF-8 path {}", path.display()))?;
        let rel = format!("{rel_prefix}/{name}");
        if path.is_dir() {
            collect_chromium_files(&path, &rel, build_files, test_files)?;
        } else if !is_output(&rel) {
            if is_test_input(&rel) {
                test_files.insert(rel, path);
            } else {
                build_files.insert(rel, path);
            }
        }
    }
    Ok(())
}

fn is_output(rel: &str) -> bool {
    rel == MANIFEST || rel == PROTOCOL_OUTPUT || rel.starts_with(HTTP_PROFILES_DIR)
}

fn is_test_input(rel: &str) -> bool {
    rel == GATE_WORKFLOW
        || rel == "chromium/surfaces.toml"
        || rel.starts_with("chromium/expectations/")
        || rel.starts_with("chromium/wpt")
        || rel.starts_with("chromium/khronos")
        || rel.starts_with("chromium/launch-args")
}

/// Computes the exact Git tree hash of `files` using an isolated temporary
/// bare Git repository so the workspace's `.git` is never mutated.
fn git_tree_hash(files: &BTreeMap<String, PathBuf>) -> Result<String> {
    let temp = tempfile::tempdir().context("creating temp dir for Git tree hash")?;
    let init = git::command(temp.path())
        .args(["init", "--bare", "-q"])
        .status()
        .context("running git init --bare")?;
    if !init.success() {
        bail!("git init --bare failed");
    }
    let index = temp.path().join("index");
    for (rel, path) in files {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let mut hash_child = git::command(temp.path())
            .env("GIT_DIR", temp.path())
            .args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("spawning git hash-object")?;
        if let Some(mut stdin) = hash_child.stdin.take() {
            use std::io::Write as _;
            stdin.write_all(&bytes)?;
        }
        let output = hash_child.wait_with_output()?;
        if !output.status.success() {
            bail!("git hash-object failed for {rel}");
        }
        let blob = String::from_utf8(output.stdout)?.trim().to_owned();
        let mode = executable_mode(path)?;
        let update = git::command(temp.path())
            .env("GIT_DIR", temp.path())
            .env("GIT_INDEX_FILE", &index)
            .args(["update-index", "--add", "--cacheinfo", mode, &blob, rel])
            .status()
            .context("running git update-index")?;
        if !update.success() {
            bail!("git update-index failed for {rel}");
        }
    }
    let tree = git::command(temp.path())
        .env("GIT_DIR", temp.path())
        .env("GIT_INDEX_FILE", &index)
        .arg("write-tree")
        .output()
        .context("running git write-tree")?;
    if !tree.status.success() {
        bail!(
            "git write-tree failed: {}",
            String::from_utf8_lossy(&tree.stderr)
        );
    }
    Ok(String::from_utf8(tree.stdout)?.trim().to_owned())
}

#[cfg(unix)]
fn executable_mode(path: &Path) -> Result<&'static str> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path)?.permissions().mode();
    Ok(if mode & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    })
}

#[cfg(not(unix))]
fn executable_mode(_path: &Path) -> Result<&'static str> {
    Ok("100644")
}

fn write_junit(path: &Path, name: &str, failure: Option<&str>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let (failures, body) = match failure {
        None => (
            0,
            format!("    <testcase classname=\"chromium-build\" name=\"{name}\" time=\"0.0\"/>\n"),
        ),
        Some(message) => {
            let escaped = message
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;");
            (
                1,
                format!(
                    "    <testcase classname=\"chromium-build\" name=\"{name}\" time=\"0.0\">\n      <failure message=\"{escaped}\"/>\n    </testcase>\n"
                ),
            )
        }
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites tests=\"1\" failures=\"{failures}\">\n  <testsuite name=\"chromium-build\" tests=\"1\" failures=\"{failures}\">\n{body}  </testsuite>\n</testsuites>\n"
    );
    std::fs::write(path, xml).with_context(|| format!("writing {}", path.display()))
}
