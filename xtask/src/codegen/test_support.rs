//! The JSON Schemas of `goodfetch-test-support`'s file types (ADR 0062),
//! such as `goodfetch-test-env`'s description. No crate takes test support
//! outside `[dev-dependencies]`, `xtask` included, so `xtask codegen` gets
//! them by running `goodfetch-test-env --print-schemas`, which prints one
//! JSON object keyed by file name and starts nothing; and the Secret
//! canaries' `gitleaks` rule, which `--print-canary-rule` prints.
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

/// Where `xtask codegen` commits the `gitleaks` rule of the Secret canaries
/// (ADR 0067), generated from their kinds, which CI's `gitleaks` step reads.
pub(super) const CANARY_RULE: &str = "test-support/fixtures/canaries/gitleaks.toml";

/// Each schema by its file name.
pub(super) fn schemas(cargo: &OsStr, root: &Path) -> Result<BTreeMap<String, Value>> {
    let output = print(cargo, root, "--print-schemas")?;
    serde_json::from_slice(&output).context("reading goodfetch-test-env's schemas")
}

/// The Secret canaries' `gitleaks` rule, as `goodfetch-test-env
/// --print-canary-rule` prints it.
pub(super) fn canary_rule(cargo: &OsStr, root: &Path) -> Result<Vec<u8>> {
    print(cargo, root, "--print-canary-rule")
}

/// What `goodfetch-test-env <flag>` prints on stdout.
fn print(cargo: &OsStr, root: &Path, flag: &str) -> Result<Vec<u8>> {
    let output = Command::new(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "goodfetch-test-support",
            "--bin",
            "goodfetch-test-env",
            "--",
            flag,
        ])
        .current_dir(root)
        .output()
        .with_context(|| format!("running goodfetch-test-env {flag}"))?;
    if !output.status.success() {
        bail!(
            "goodfetch-test-env {flag} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}
