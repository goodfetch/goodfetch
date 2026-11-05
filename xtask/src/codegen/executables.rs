//! Both Products' command lines, which the `command:` Features and the
//! table of Foxhound's names read (ADRs 0072 and 0087). On the workspace
//! `xtask` is built from, `xtask` links Collie's executable crate, whose
//! entry builds each Product's clap `Command`, so `xtask codegen --check`
//! starts no process for them. Another workspace, such as a test's fixture,
//! gives them by running Collie's `commands` example, and one without
//! Collie's executable crate has no command.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use goodfetch_collie::executable::command;
use goodfetch_protocol::catalog::Product;

use super::emitter;
use crate::features::{CommandLines, CommandTree};

/// Collie's executable crate, whose `executable` module both Products'
/// executables start from (ADR 0087).
const COLLIE: &str = "crates/goodfetch-collie";

pub(super) fn command_lines(cargo: &OsStr, root: &Path) -> Result<Option<CommandLines>> {
    if emitter::built_from(root) {
        return Ok(Some(CommandLines {
            collie: CommandTree::of(&command(Product::Collie)),
            foxhound: CommandTree::of(&command(Product::Foxhound)),
        }));
    }
    if !root.join(COLLIE).is_dir() {
        return Ok(None);
    }
    run_example(cargo, root).map(Some)
}

fn run_example(cargo: &OsStr, root: &Path) -> Result<CommandLines> {
    let folder = tempfile::tempdir().context("creating a folder for the command lines")?;
    let output = crate::child_cargo::command(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "goodfetch-collie",
            "--features",
            "executable",
            "--example",
            "commands",
            "--",
        ])
        .arg(folder.path())
        .current_dir(root)
        .output()
        .context("running goodfetch-collie's commands example")?;
    if !output.status.success() {
        bail!(
            "goodfetch-collie's commands example failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = folder.path().join("commands.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).context("reading goodfetch-collie's command lines")
}
