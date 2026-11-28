//! Launch-argument codegen for the Chromium build gate's WPT runs (ADR 0074;
//! ticket #395): runs `goodfetch-page`'s `launch-arguments` example during
//! `xtask codegen` so the committed `chromium/launch-args.json` comes from the
//! engine's own launch-argument builder and Fingerprint methods (`none` and
//! `js_injection`, with a fixed Identity profile seed; `engine` joins in #969),
//! never written by hand.
//!
//! Per ADR 0072's 2026-11-06 amendment, `xtask codegen --check` never builds
//! `goodfetch-page`: it verifies `chromium/launch-args.json` against the input
//! recipe and output digest recorded in `digests.json` (catching both hand
//! edits and source changes), while `goodfetch-page`'s own unit test verifies
//! the committed file against `wpt_launch_args(WPT_IDENTITY_SEED)`.
//!
//! A minimal fixture workspace without `crates/goodfetch-page` and without
//! `chromium/launch-args.json` skips this file; if either exists without
//! `crates/goodfetch-page/examples/launch-arguments.rs`, codegen and `--check`
//! fail rather than skipping silently.
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

/// The owning crate folder.
const PAGE_CRATE: &str = "crates/goodfetch-page";

/// The example in `goodfetch-page` that emits the engine's launch arguments.
const EXAMPLE_SOURCE: &str = "crates/goodfetch-page/examples/launch-arguments.rs";

/// Additional `goodfetch-page` sources whose changes alter the generated
/// launch arguments, hashed into the digest recipe when present.
const ENGINE_SOURCES: [&str; 2] = [
    "crates/goodfetch-page/src/cdp/launch/arguments.rs",
    "crates/goodfetch-page/src/fingerprint_method.rs",
];

/// The file name the example writes inside the temporary folder.
const WRITTEN: &str = "launch-args.json";

/// Returns the deterministic input recipe bytes hashed into `digests.json` for
/// `chromium/launch-args.json`, or `None` when `root` has neither
/// `crates/goodfetch-page` nor `chromium/launch-args.json` (such as the
/// minimal `codegen` fixture). Fails if either exists without
/// `crates/goodfetch-page/examples/launch-arguments.rs`.
pub(super) fn recipe(root: &Path) -> Result<Option<Vec<u8>>> {
    let has_crate = root.join(PAGE_CRATE).is_dir();
    let has_output = root.join(LAUNCH_ARGS).exists();
    if !has_crate && !has_output {
        return Ok(None);
    }
    let example_path = root.join(EXAMPLE_SOURCE);
    if !example_path.is_file() {
        bail!("missing {EXAMPLE_SOURCE}: cannot generate or check {LAUNCH_ARGS}");
    }
    let mut recipe = Vec::new();
    append_source(&mut recipe, EXAMPLE_SOURCE, &example_path)?;
    for rel in ENGINE_SOURCES {
        let path = root.join(rel);
        if path.is_file() {
            append_source(&mut recipe, rel, &path)?;
        }
    }
    Ok(Some(recipe))
}

fn append_source(recipe: &mut Vec<u8>, rel: &str, path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    recipe.extend_from_slice(rel.as_bytes());
    recipe.push(b'\0');
    recipe.extend_from_slice(&bytes);
    recipe.push(b'\0');
    Ok(())
}

/// Generates `chromium/launch-args.json` by running `goodfetch-page`'s
/// `launch-arguments` example. Called only by `xtask codegen` (write), never
/// by `xtask codegen --check`.
pub(super) fn generate(cargo: &OsStr, root: &Path) -> Result<Vec<u8>> {
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
    super::json_text(&value)
}
