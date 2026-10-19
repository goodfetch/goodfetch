//! The JSON Schemas of `goodfetch-test-support`'s file types (ADR 0062),
//! such as `goodfetch-test-env`'s description. No crate takes test support
//! outside `[dev-dependencies]`, `xtask` included, so `xtask codegen` gets
//! them by running `goodfetch-test-env --print-schemas`, which prints one
//! JSON object keyed by file name and starts nothing.
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

/// Where `xtask codegen` commits them, among the shared fixtures.
pub(super) const FOLDER: &str = "test-support/fixtures/schemas";

/// Each schema by its file name.
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
    serde_json::from_slice(&output.stdout).context("reading goodfetch-test-env's schemas")
}
