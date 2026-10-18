//! The Cargo workspace's third-party crates, every feature on, read from
//! `cargo metadata`: the graph cargo-deny checks.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process and blocking-fs bans target async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::entry::{Ecosystem, Entry, Usage};

/// crates.io, by either of its index protocols.
const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: BTreeSet<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    license: Option<String>,
    /// Where Cargo got it; `None` for a folder.
    source: Option<String>,
    manifest_path: PathBuf,
}

/// Every crate of the workspace's graph that is not one of its members.
pub(super) fn entries(cargo: &OsStr, root: &Path) -> Result<Vec<Entry>> {
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
        ])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).context("reading cargo metadata's output")?;
    Ok(metadata
        .packages
        .into_iter()
        .filter(|package| !metadata.workspace_members.contains(&package.id))
        .map(|package| Entry {
            ecosystem: Ecosystem::Cargo,
            source: source(&package, root),
            name: package.name,
            version: package.version,
            license: package.license,
            usage: Usage::Unmodified,
        })
        .collect())
}

/// Its crates.io page, its other registry or repository, or its folder.
fn source(package: &Package, root: &Path) -> String {
    match package.source.as_deref() {
        Some(source) if CRATES_IO.contains(&source) => format!(
            "https://crates.io/crates/{}/{}",
            package.name, package.version
        ),
        Some(source) => source
            .split_once('+')
            .map_or(source, |(_, location)| location)
            .to_owned(),
        None => {
            // Cargo's paths are canonical; the root may not be.
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
            let folder = package.manifest_path.parent().unwrap_or(&root);
            folder
                .strip_prefix(&root)
                .unwrap_or(folder)
                .to_string_lossy()
                .replace('\\', "/")
        }
    }
}
