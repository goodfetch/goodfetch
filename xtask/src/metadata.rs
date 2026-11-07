//! The Cargo workspace's own packages, read from `cargo metadata --no-deps`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) manifest_path: PathBuf,
    pub(crate) dependencies: Vec<Dependency>,
    pub(crate) targets: Vec<Target>,
    /// `[package.metadata]`.
    #[serde(default)]
    metadata: Option<PackageMetadata>,
}

impl Package {
    /// The workspace crates whose executables the package's tests run, from
    /// `[package.metadata.goodfetch] runs`.
    pub(crate) fn runs(&self) -> Vec<String> {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.goodfetch.as_ref())
            .map(|goodfetch| goodfetch.runs.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct PackageMetadata {
    goodfetch: Option<GoodfetchMetadata>,
}

/// `[package.metadata.goodfetch]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoodfetchMetadata {
    #[serde(default)]
    runs: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct Dependency {
    pub(crate) name: String,
    /// `None` for a normal dependency, `dev` or `build` otherwise.
    pub(crate) kind: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Target {
    pub(crate) name: String,
    pub(crate) kind: Vec<String>,
    pub(crate) src_path: PathBuf,
    /// The features the target needs, such as a Product's `executable`.
    #[serde(default, rename = "required-features")]
    pub(crate) required_features: Vec<String>,
}

/// Every member of the workspace at `root`.
pub(crate) fn packages(cargo: &OsStr, root: &Path) -> Result<Vec<Package>> {
    read(cargo, root).map(|metadata| metadata.packages)
}

/// The workspace's target folder, as Cargo resolves it (`CARGO_TARGET_DIR`
/// included).
pub(crate) fn target_directory(cargo: &OsStr, root: &Path) -> Result<PathBuf> {
    read(cargo, root).map(|metadata| metadata.target_directory)
}

fn read(cargo: &OsStr, root: &Path) -> Result<Metadata> {
    let output = crate::child_cargo::command(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("reading cargo metadata's output")
}
