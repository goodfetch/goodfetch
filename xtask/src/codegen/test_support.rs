//! The JSON Schemas of `goodfetch-test-support`'s file types (ADR 0062),
//! such as `goodfetch-test-env`'s description and the Scripted engine's
//! script format (ADR 0069), and the Secret canaries' `gitleaks` rule
//! (ADR 0067). No crate takes test support outside `[dev-dependencies]`,
//! `xtask` included, so `xtask codegen` gets them by running
//! `goodfetch-test-env`: `--print-schemas` prints one JSON object keyed by
//! each schema's path under `test-support/fixtures/`, and
//! `--print-canary-rule` the rule; neither starts anything.
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

/// The shared fixtures, under which each schema's path is given.
pub(super) const FIXTURES: &str = "test-support/fixtures";

/// The folder that holds only schemas `xtask codegen` writes; the script
/// format's sits among the engine scripts instead.
pub(super) const FOLDER: &str = "test-support/fixtures/schemas";

/// Where `xtask codegen` commits the `gitleaks` rule of the Secret canaries
/// (ADR 0067), generated from their kinds, which CI's `gitleaks` step reads.
pub(super) const CANARY_RULE: &str = "test-support/fixtures/canaries/gitleaks.toml";

/// Each schema by its path relative to the root.
pub(super) fn schemas(cargo: &OsStr, root: &Path) -> Result<BTreeMap<String, Value>> {
    let output = print(cargo, root, "--print-schemas")?;
    let schemas: BTreeMap<String, Value> =
        serde_json::from_slice(&output).context("reading goodfetch-test-env's schemas")?;
    schemas
        .into_iter()
        .map(|(path, schema)| Ok((format!("{FIXTURES}/{}", fixture_path(&path)?), schema)))
        .collect()
}

/// The Secret canaries' `gitleaks` rule, as `goodfetch-test-env
/// --print-canary-rule` prints it.
pub(super) fn canary_rule(cargo: &OsStr, root: &Path) -> Result<Vec<u8>> {
    print(cargo, root, "--print-canary-rule")
}

/// What `goodfetch-test-env <flag>` prints on stdout. The binary is named
/// alone, without its package, so its dependencies resolve their features
/// as the test tools' build of it resolves them, and it reuses that build.
fn print(cargo: &OsStr, root: &Path, flag: &str) -> Result<Vec<u8>> {
    let output = crate::child_cargo::command(cargo)
        .args(["run", "--quiet", "--bin", "goodfetch-test-env", "--", flag])
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
