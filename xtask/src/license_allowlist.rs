//! ADR 0018's license allowlist, `license-allowlist.toml`: the one list every
//! license check reads, cargo-deny's copy in `deny.toml` (which `xtask
//! codegen` writes) and the third-party notice inventory's (`xtask notices`).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

/// The allowlist, at the repository's root.
pub(crate) const PATH: &str = "license-allowlist.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    license: Vec<License>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct License {
    /// An SPDX license identifier, with its exception when it has one
    /// (`Apache-2.0 WITH LLVM-exception`).
    pub(crate) id: String,
    /// The one use or package the license is allowed for, when it is not
    /// allowed for all.
    pub(crate) only: Option<Only>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub(crate) enum Only {
    Use(Use),
    Package(Package),
}

/// One npm or Python package, the only one the license is allowed for. No
/// crate can be one: cargo-deny reads the list's unscoped entries.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct Package {
    pub(crate) ecosystem: PackageEcosystem,
    /// Its name, as its registry and lockfile spell it.
    pub(crate) package: String,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PackageEcosystem {
    Npm,
    Python,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Use {
    /// Files used as their authors published them (MPL-2.0, ADR 0018); a
    /// crate or package from its registry is.
    UnmodifiedFiles,
    /// The compiled form of Filter lists embedded in the executable (CC
    /// BY-SA, ADRs 0018 and 0047); never code.
    CompiledFilterLists,
}

/// Every license of the allowlist at `root`, in its order.
pub(crate) fn read(root: &Path) -> Result<Vec<License>> {
    let path = root.join(PATH);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let allowlist: Allowlist = toml::from_str(&text).with_context(|| format!("reading {PATH}"))?;
    Ok(allowlist.license)
}
