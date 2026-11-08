//! The JSON Schema bundle and catalog file of `goodfetch-protocol`'s types
//! (ADR 0038). On the workspace `xtask` is built from, `xtask` links that
//! crate and emits them itself (`emit`), so `xtask codegen --check` starts
//! no process. Another workspace, such as a test's fixture with its own
//! `goodfetch-protocol`, is emitted by running that crate's `emit` example:
//! the Rust toolchain, never a generator.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// The two files the emitter wrote, as it wrote them.
pub(super) struct Emitted {
    pub(super) bundle: Vec<u8>,
    pub(super) catalog: Vec<u8>,
}

/// The workspace this `xtask` was built from.
const BUILT_FROM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

pub(super) fn emit(cargo: &OsStr, root: &Path) -> Result<Emitted> {
    if built_from(root) {
        return linked();
    }
    run_example(cargo, root)
}

/// Whether `root` is the workspace this `xtask` was built from, whose crates
/// it links.
pub(super) fn built_from(root: &Path) -> bool {
    let canonical = |path: &Path| path.canonicalize().ok();
    canonical(root).is_some() && canonical(root) == canonical(Path::new(BUILT_FROM))
}

/// The files as the `emit` example writes them: pretty JSON with a final
/// newline.
fn linked() -> Result<Emitted> {
    let emitted = super::emit::emit();
    let text = |value: &serde_json::Value| -> Result<Vec<u8>> {
        let mut text = serde_json::to_string_pretty(value)?;
        text.push('\n');
        Ok(text.into_bytes())
    };
    Ok(Emitted {
        bundle: text(&emitted.bundle)?,
        catalog: text(&emitted.catalog)?,
    })
}

fn run_example(cargo: &OsStr, root: &Path) -> Result<Emitted> {
    let folder = tempfile::tempdir().context("creating a folder for the emitter")?;
    let output = crate::child_cargo::command(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "goodfetch-protocol",
            "--example",
            "emit",
            "--",
        ])
        .arg(folder.path())
        .current_dir(root)
        .output()
        .context("running goodfetch-protocol's emitter")?;
    if !output.status.success() {
        bail!(
            "goodfetch-protocol's emitter failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let read = |name: &str| {
        std::fs::read(folder.path().join(name))
            .with_context(|| format!("reading the emitted {name}"))
    };
    Ok(Emitted {
        bundle: read("bundle.json")?,
        catalog: read("catalog.json")?,
    })
}
