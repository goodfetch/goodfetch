//! Launch-argument codegen for the Chromium build gate's WPT runs (ADR 0074;
//! ticket #395): runs `goodfetch-page`'s `launch-arguments` example so the
//! committed `chromium/launch-args.json` comes from the engine's own
//! launch-argument builder and Fingerprint methods (`none` and `js_injection`,
//! with a fixed Identity profile seed; `engine` joins in #969), never written
//! by hand. `xtask` builds no engine crate (`xtask dependency-direction`), so
//! it invokes the example through `cargo run` on both `codegen` and
//! `codegen --check`.
//!
//! A workspace without `goodfetch-page`'s example, such as the minimal
//! `codegen` fixture, skips this file.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

/// Where `xtask codegen` commits the WPT launch arguments (one of the Chromium
/// build gate's test inputs, ADR 0074).
pub(super) const LAUNCH_ARGS: &str = "chromium/launch-args.json";

/// The example in `goodfetch-page` that emits the engine's launch arguments.
const EXAMPLE_SOURCE: &str = "crates/goodfetch-page/examples/launch-arguments.rs";

/// The file name the example writes inside the temporary folder.
const WRITTEN: &str = "launch-args.json";

/// Generates `chromium/launch-args.json` by running `goodfetch-page`'s
/// `launch-arguments` example, or returns `None` when `root` does not hold
/// that example (such as a minimal fixture workspace).
pub(super) fn generate(cargo: &OsStr, root: &Path) -> Result<Option<Vec<u8>>> {
    if !root.join(EXAMPLE_SOURCE).is_file() {
        return Ok(None);
    }
    let folder = tempfile::tempdir().context("creating a folder for WPT launch arguments")?;
    let target = folder.path().join(WRITTEN);
    let output = crate::child_cargo::command(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "goodfetch-page",
            "--example",
            "launch-arguments",
            "--",
        ])
        .arg(&target)
        .current_dir(root)
        .output()
        .context("running goodfetch-page's launch-arguments example")?;
    if !output.status.success() {
        bail!(
            "goodfetch-page's launch-arguments example failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let raw = std::fs::read(&target).with_context(|| format!("reading {}", target.display()))?;
    let value: Value =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", target.display()))?;
    Ok(Some(super::json_text(&value)?))
}
