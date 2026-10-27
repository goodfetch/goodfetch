//! The namespaces of the commands `goodfetch-cli` generates from Foxhound's
//! catalog, which holds Collie's, each marked by the Product whose catalog
//! first gives it a command, as a Rust constant in `goodfetch-cli`, which a
//! start reads instead of walking the catalog (ADRs 0071 and 0099). On the workspace
//! `xtask` is built from, `xtask` links `goodfetch-cli` and writes what its
//! catalog gives; another workspace, such as a test's fixture, has no
//! `goodfetch-cli` and no such file.
//!
//! The command lines `xtask codegen` reads are built with the namespaces
//! `xtask` was compiled with, so a run that changes them runs `xtask
//! codegen` once more, compiled with the new ones.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use super::{Generators, emitter};

/// The crate that reads the constant.
const CRATE: &str = "crates/goodfetch-cli";

/// Where `goodfetch-cli`'s generated files live.
pub(super) const FOLDER: &str = "crates/goodfetch-cli/generated";

/// The file of the namespaces.
const NAMESPACES: &str = "crates/goodfetch-cli/generated/namespaces.rs";

/// The file of the namespaces and its text, none for a workspace without
/// `goodfetch-cli`.
pub(super) fn namespaces(root: &Path) -> Result<Option<(String, Vec<u8>)>> {
    if emitter::built_from(root) {
        return Ok(Some((
            NAMESPACES.to_owned(),
            goodfetch_cli::generated::namespaces_source().into_bytes(),
        )));
    }
    if root.join(CRATE).is_dir() {
        bail!("only the `xtask` built from a workspace writes its {NAMESPACES}");
    }
    Ok(None)
}

/// The namespaces file as `root` holds it, none when it holds none.
pub(super) fn written(root: &Path) -> Option<Vec<u8>> {
    std::fs::read(root.join(NAMESPACES)).ok()
}

/// Runs `xtask codegen` again at `root`, compiled with the namespaces this
/// run wrote, so the command lines it reads hold them.
pub(super) fn rerun(cargo: &OsStr, root: &Path, generators: &Generators) -> Result<()> {
    let absolute = |path: &Path| {
        std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))
    };
    let status = Command::new(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "xtask",
            "--",
            "codegen",
            "--json2ts",
        ])
        .arg(absolute(&generators.json2ts)?)
        .arg("--datamodel-codegen")
        .arg(absolute(&generators.datamodel_codegen)?)
        .current_dir(root)
        .status()
        .context("running `xtask codegen` again")?;
    if !status.success() {
        bail!("`xtask codegen`, run again with the new {NAMESPACES}, failed");
    }
    Ok(())
}
