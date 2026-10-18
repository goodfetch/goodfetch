//! The lockfile half of ADR 0062's mocking bans: `uv.lock` resolves every
//! dependency group of the uv workspace, so a mocking library in it is in some
//! group. (cargo-deny, oxlint and ruff hold the other bans in their configs.)
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

const LOCKFILE: &str = "uv.lock";

/// Python mocking libraries no dependency group may hold (TESTING.md).
const BANNED: [&str; 1] = ["pytest-mock"];

#[derive(Deserialize)]
struct Lockfile {
    #[serde(rename = "package", default)]
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let path = root.join(LOCKFILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let lockfile: Lockfile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(lockfile
        .packages
        .iter()
        .filter(|package| BANNED.contains(&package.name.as_str()))
        .map(|package| {
            Finding::new(
                "mocking-library",
                LOCKFILE,
                format!(
                    "`{}` mocks owned code, which ADR 0062 refuses",
                    package.name
                ),
            )
        })
        .collect())
}
