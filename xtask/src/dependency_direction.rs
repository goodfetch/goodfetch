//! The dependency-direction check over the Cargo workspace's manifests: the
//! directions STYLE.md's crate rules make the compiler enforce. It holds
//! `goodfetch-test-support`'s rule (ADR 0062): the crate depends on no
//! Goodfetch crate, and every other crate takes it only as a dev-dependency.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::Finding;

const TEST_SUPPORT: &str = "goodfetch-test-support";

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    /// `None` for a normal dependency, `dev` or `build` otherwise.
    kind: Option<String>,
}

fn workspace_packages(cargo: &OsStr, root: &Path) -> Result<Vec<Package>> {
    let output = Command::new(cargo)
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
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).context("reading cargo metadata's output")?;
    Ok(metadata.packages)
}

pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    let packages = workspace_packages(cargo, root)?;
    let is_member = |name: &str| packages.iter().any(|package| package.name == name);
    let mut findings = Vec::new();
    for package in &packages {
        if package.name == TEST_SUPPORT {
            if let Some(dependency) = package
                .dependencies
                .iter()
                .find(|dependency| is_member(&dependency.name))
            {
                findings.push(Finding::new(
                    "test-support-depends-on-goodfetch",
                    &package.name,
                    format!("it depends on the workspace crate `{}`", dependency.name),
                ));
            }
        } else if package.dependencies.iter().any(|dependency| {
            dependency.name == TEST_SUPPORT && dependency.kind.as_deref() != Some("dev")
        }) {
            findings.push(Finding::new(
                "test-support-not-dev-only",
                &package.name,
                format!("it takes `{TEST_SUPPORT}` outside `[dev-dependencies]`"),
            ));
        }
    }
    Ok(findings)
}
