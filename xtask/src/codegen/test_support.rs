//! The JSON Schemas of `goodfetch-test-support`'s file types (ADR 0062),
//! such as `goodfetch-test-env`'s description and the Scripted engine's
//! script format (ADR 0069). No crate takes test support outside
//! `[dev-dependencies]`, `xtask` included, so `xtask codegen` gets them by
//! running `goodfetch-test-env --print-schemas`, which prints one JSON
//! object keyed by each schema's path under `test-support/fixtures/` and
//! starts nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

/// The shared fixtures, under which each schema's path is given.
pub(super) const FIXTURES: &str = "test-support/fixtures";

/// The folder that holds only schemas `xtask codegen` writes; the script
/// format's sits among the engine scripts instead.
pub(super) const FOLDER: &str = "test-support/fixtures/schemas";

/// Each schema by its path relative to the root.
pub(super) fn schemas(cargo: &OsStr, root: &Path) -> Result<BTreeMap<String, Value>> {
    let output = Command::new(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "goodfetch-test-support",
            "--bin",
            "goodfetch-test-env",
            "--",
            "--print-schemas",
        ])
        .current_dir(root)
        .output()
        .context("running goodfetch-test-env --print-schemas")?;
    if !output.status.success() {
        bail!(
            "goodfetch-test-env --print-schemas failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let schemas: BTreeMap<String, Value> =
        serde_json::from_slice(&output.stdout).context("reading goodfetch-test-env's schemas")?;
    schemas
        .into_iter()
        .map(|(path, schema)| Ok((format!("{FIXTURES}/{}", fixture_path(&path)?), schema)))
        .collect()
}

/// A path `goodfetch-test-env` printed, once checked to name a file in a
/// folder of the shared fixtures, with `/` and nothing that leaves them.
fn fixture_path(path: &str) -> Result<&str> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains('\\'))
    {
        bail!(
            "goodfetch-test-env printed a schema at {path}, which is no file in a folder of {FIXTURES}"
        );
    }
    Ok(path)
}
