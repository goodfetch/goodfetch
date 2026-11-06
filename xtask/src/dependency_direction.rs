//! The dependency-direction check over the Cargo workspace's manifests: the
//! directions STYLE.md's crate rules make the compiler enforce.
//! - `goodfetch-test-support` depends on no Goodfetch crate, and every other
//!   crate takes it only as a dev-dependency (ADR 0062).
//! - The Proving ground depends on no Goodfetch crate but
//!   `goodfetch-test-support`, so on no engine crate, nor on
//!   `goodfetch-conformance` or `goodfetch-protocol` (ADR 0032): the oracle
//!   stays independent of what it judges.
//! - `goodfetch-conformance` depends on no Goodfetch crate but
//!   `goodfetch-protocol` (and `goodfetch-test-support` for its tests): the
//!   harness tests only through the interface.
//!
//!
//! `goodfetch-workspace-hack`, which every member takes (ADR 0094), holds no
//! code, so no rule counts it as a Goodfetch crate.
//!
//! The workspace-wide direction of ADR 0087 is cargo-deny's (`deny.toml`).

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::check::Finding;
use crate::metadata::{self, Dependency, Package};

const TEST_SUPPORT: &str = "goodfetch-test-support";
const CONFORMANCE: &str = "goodfetch-conformance";
const PROTOCOL: &str = "goodfetch-protocol";
const WORKSPACE_HACK: &str = "goodfetch-workspace-hack";
/// The Proving ground's manifest, from the workspace's root (ADR 0032).
const PROVING_GROUND_MANIFEST: &str = "proving-ground/Cargo.toml";

pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    let packages = metadata::packages(cargo, root)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    let is_member = |dependency: &Dependency| {
        dependency.name != WORKSPACE_HACK
            && packages
                .iter()
                .any(|package| package.name == dependency.name)
    };
    let mut findings = Vec::new();
    for package in &packages {
        let members = || package.dependencies.iter().filter(|d| is_member(d));
        if package.name == TEST_SUPPORT {
            if let Some(dependency) = members().next() {
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
        if is_proving_ground(package, &root)
            && let Some(dependency) = members().find(|d| d.name != TEST_SUPPORT)
        {
            findings.push(Finding::new(
                "proving-ground-depends-on-goodfetch",
                &package.name,
                format!(
                    "the Proving ground depends on `{}`; it takes no Goodfetch crate but `{TEST_SUPPORT}`",
                    dependency.name
                ),
            ));
        }
        if package.name == CONFORMANCE
            && let Some(dependency) =
                members().find(|d| d.name != PROTOCOL && d.name != TEST_SUPPORT)
        {
            findings.push(Finding::new(
                "conformance-depends-on-more-than-protocol",
                &package.name,
                format!(
                    "it depends on `{}`; it takes no Goodfetch crate but `{PROTOCOL}`",
                    dependency.name
                ),
            ));
        }
    }
    Ok(findings)
}

fn is_proving_ground(package: &Package, root: &Path) -> bool {
    package.manifest_path == root.join(PROVING_GROUND_MANIFEST)
}
