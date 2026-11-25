//! The Product version the released crates carry (ADR 0030), and what else
//! the root manifest gives every crate.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::products;

/// The Product version the released crates carry, as Collie's executable
/// crate states it: the root manifest's `[workspace.package] version` while
/// it takes the workspace's, its own once `xtask release prepare` set it,
/// the internal crates keeping the workspace's `0.0.x` (ADR 0030).
pub(crate) fn read(root: &Path) -> Result<String> {
    let path = root.join(products::COLLIE.folder).join("Cargo.toml");
    match manifest(&path)?
        .get("package")
        .and_then(|package| package.get("version"))
    {
        Some(toml::Value::String(version)) => Ok(version.clone()),
        _ => workspace_package(root, "version"),
    }
}

/// The root manifest's `[workspace.package]` string `key`, which every
/// crate takes.
pub(crate) fn workspace_package(root: &Path, key: &str) -> Result<String> {
    let path = root.join("Cargo.toml");
    manifest(&path)?
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get(key))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("{} has no `[workspace.package] {key}`", path.display()))
}

fn manifest(path: &Path) -> Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
}

#[cfg(test)]
mod tests {
    use goodfetch_test_support::TestEnv;
    use rstest::rstest;

    use super::read;
    use crate::products;

    /// Both SDKs' `PRODUCT_VERSION` is the released crates' version: the
    /// workspace's `0.0.x` until `xtask release prepare` sets theirs, and
    /// the new version after, though the workspace's stays (issue 2876).
    #[rstest]
    #[case::before_a_release("version.workspace = true", "0.0.0")]
    #[case::after_its_preparation("version = \"1.4.0-rc.2\"", "1.4.0-rc.2")]
    fn the_product_version_is_the_released_crates(#[case] version: &str, #[case] expected: &str) {
        let env = TestEnv::new();
        let root = env.folder();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\n\n[workspace.package]\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        let collie = root.join(products::COLLIE.folder);
        std::fs::create_dir_all(&collie).unwrap();
        std::fs::write(
            collie.join("Cargo.toml"),
            format!("[package]\nname = \"goodfetch-collie\"\n{version}\n"),
        )
        .unwrap();

        assert_eq!(read(root).unwrap(), expected);
    }
}
