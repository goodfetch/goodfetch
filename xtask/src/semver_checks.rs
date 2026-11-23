//! The API breakage check of the Rust crates (ADRs 0030, 0069, 0094 and
//! 0095, issue 148): cargo-semver-checks on `goodfetch-collie-client`,
//! `goodfetch-collie` and `goodfetch-foxhound`, each with every feature, so
//! the testing kits (`test-util`'s `goodfetch_collie::testing` and the
//! client's `testing` module) are checked with the rest, against the commit
//! `compat/api.toml` names as the previous release, which `xtask api-check`
//! compares the contracts with.
//!
//! Each side's rustdoc JSON is built here, with the dependencies its own
//! `Cargo.lock` pins, the previous release's from that commit's tree, which
//! `git archive` writes under the target folder. cargo-semver-checks would
//! otherwise build each crate in a workspace of its own, which resolves the
//! newest dependencies instead, and finds the package by name anywhere under
//! the repository, where test fixtures and proofs hold copies of the crates.
//!
//! cargo-semver-checks compares the two as for a minor release: a lint that
//! requires a major version is a break, one a minor allows, such as a
//! deprecation, passes, and an addition fails no lint. Each crate is
//! compared on its own, so each failed lint is that crate's: its subject is
//! `rust <crate> <lint>`, which a `compat/api.toml` announcement names to
//! pass it, and the items it failed in are its message.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};

use crate::api_baseline::{self, Changes, RUST};

/// The crates under semver: each Product's, and the Collie client's, which
/// both Products' SDKs share (ADR 0095).
const CRATES: [&str; 3] = [
    "goodfetch-collie-client",
    "goodfetch-collie",
    "goodfetch-foxhound",
];

/// The installation the check names when cargo-semver-checks is missing:
/// the CI image's version.
const INSTALL: &str = "cargo install --locked cargo-semver-checks@0.51.0";

pub(crate) fn run(root: &Path, cargo: &OsStr) -> Result<ExitCode> {
    let baseline = api_baseline::baseline(root)?;
    let target = crate::metadata::target_directory(cargo, root)?.join("semver-checks");
    let previous = previous_tree(root, &baseline.commit, &target)?;
    let mut entries: BTreeMap<String, Changes> = BTreeMap::new();
    for name in CRATES {
        let current = rustdoc(cargo, root, name, &target, "current")?;
        let before = rustdoc(cargo, &previous, name, &target, "previous")?;
        let output = crate::child_cargo::command(cargo)
            .current_dir(root)
            .args([
                "semver-checks",
                "check-release",
                "--color",
                "never",
                "--release-type",
                "minor",
            ])
            .arg("--current-rustdoc")
            .arg(&current)
            .arg("--baseline-rustdoc")
            .arg(&before)
            .output()
            .with_context(|| format!("running cargo-semver-checks on {name}"))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no such command: `semver-checks`") {
            bail!("cargo-semver-checks is not installed: `{INSTALL}`");
        }
        let failures = failures(&String::from_utf8_lossy(&output.stdout));
        if !output.status.success() && failures.is_empty() {
            bail!("cargo-semver-checks failed on {name} with no lint failed:\n{stderr}");
        }
        for (lint, items) in failures {
            let changes = entries.entry(format!("{RUST}{name} {lint}")).or_default();
            for item in items {
                changes.breaks(&lint, item);
            }
        }
    }
    Ok(baseline
        .verdict(|entry| entry.starts_with(RUST), &entries)
        .report())
}

/// The tree of `commit` under `target`, written by `git archive` the first
/// time and read as it is after: a commit's tree never changes. It is
/// written beside its place and renamed into it, so a check stopped halfway
/// or run twice at once leaves no partial tree there.
fn previous_tree(root: &Path, commit: &str, target: &Path) -> Result<PathBuf> {
    let tree = target.join(format!("previous-{commit}"));
    if tree.is_dir() {
        return Ok(tree);
    }
    let partial = target.join(format!("previous-{commit}.{}", std::process::id()));
    if partial.exists() {
        std::fs::remove_dir_all(&partial)
            .with_context(|| format!("removing {}", partial.display()))?;
    }
    api_baseline::archive(root, commit, &[], &partial)?;
    match std::fs::rename(&partial, &tree) {
        Ok(()) => Ok(tree),
        // Another check wrote the same tree first.
        Err(_) if tree.is_dir() => {
            std::fs::remove_dir_all(&partial)
                .with_context(|| format!("removing {}", partial.display()))?;
            Ok(tree)
        }
        Err(error) => Err(error).with_context(|| format!("renaming into {}", tree.display())),
    }
}

/// The rustdoc JSON of the library of `name` in the workspace at
/// `workspace`, with every feature and the dependencies its `Cargo.lock`
/// pins, as `<target>/<crate>-<side>.json`. Stable rustdoc writes JSON
/// behind `-Z unstable-options`, which `RUSTC_BOOTSTRAP` opens, as
/// cargo-semver-checks itself does.
fn rustdoc(
    cargo: &OsStr,
    workspace: &Path,
    name: &str,
    target: &Path,
    side: &str,
) -> Result<PathBuf> {
    let output = crate::child_cargo::command(cargo)
        .current_dir(workspace)
        .env("RUSTC_BOOTSTRAP", "1")
        .args([
            "rustdoc",
            "--locked",
            "--lib",
            "--all-features",
            "--package",
            name,
        ])
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(target)
        .args(["--", "-Z", "unstable-options", "--output-format", "json"])
        .output()
        .with_context(|| format!("running cargo rustdoc on {name} ({side})"))?;
    if !output.status.success() {
        bail!(
            "cargo rustdoc on {name} ({side}) failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let crate_name = name.replace('-', "_");
    let written = target.join("doc").join(format!("{crate_name}.json"));
    let kept = target.join(format!("{crate_name}-{side}.json"));
    std::fs::rename(&written, &kept)
        .with_context(|| format!("moving {} to {}", written.display(), kept.display()))?;
    Ok(kept)
}

/// Each failed lint of a cargo-semver-checks report and the items it failed
/// in: a `--- failure <lint>: <summary> ---` heading, then, after its
/// description, `Failed in:` and one indented line per item, up to a blank
/// line. A warning's heading starts `--- warning`, and fails nothing.
fn failures(report: &str) -> BTreeMap<String, Vec<String>> {
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut lint: Option<String> = None;
    let mut listing = false;
    for line in report.lines() {
        if let Some(heading) = line.strip_prefix("--- ") {
            listing = false;
            lint = heading
                .strip_prefix("failure ")
                .and_then(|failure| failure.split_once(':'))
                .map(|(name, _)| name.to_owned());
            if let Some(name) = &lint {
                failures.entry(name.clone()).or_default();
            }
        } else if line == "Failed in:" {
            listing = true;
        } else if line.trim().is_empty() {
            listing = false;
        } else if listing && let Some(name) = &lint {
            failures
                .entry(name.clone())
                .or_default()
                .push(line.trim().to_owned());
        }
    }
    failures
}
