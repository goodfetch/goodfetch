//! The dependency-direction check over the Cargo workspace's manifests: the
//! directions STYLE.md's crate rules make the compiler enforce. It holds
//! `goodfetch-test-support`'s rule (ADR 0062): the crate depends on no
//! Goodfetch crate, and every other crate takes it only as a dev-dependency.

use std::ffi::OsStr;
use std::path::Path;

use anyhow::Result;

use crate::check::Finding;
use crate::metadata;

const TEST_SUPPORT: &str = "goodfetch-test-support";

pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    let packages = metadata::packages(cargo, root)?;
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
