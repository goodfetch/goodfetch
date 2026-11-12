//! The Console of a tree that predates the Console's removal (ADR 0137).
//! The base `xtask perf build` measures and the base `xtask stress select`
//! lists are built from their own checkouts; such a tree's executables
//! embed a Console build and do not compile without it. The tree builds its
//! own, from its own packages with its own `xtask console build`: this
//! xtask has no Console to build it with.
//! Issue 2516 deletes this module and its two callers once no such tree is
//! checked out.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// The source of a tree's `xtask console build`.
const COMMAND_SOURCE: &str = "xtask/src/console.rs";
const PNPM: &str = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };

/// Whether the tree checked out at `tree` still has a Console to build.
pub(crate) fn in_tree(tree: &Path) -> bool {
    tree.join(COMMAND_SOURCE).is_file()
}

/// Installs `tree`'s pnpm packages as its lockfile names them, which its
/// Console builds from.
pub(crate) fn install(tree: &Path) -> Command {
    let mut command = Command::new(PNPM);
    command
        .current_dir(tree)
        .args(["install", "--frozen-lockfile"]);
    command
}

/// Runs `tree`'s own `xtask console build`, built into `target_directory`.
pub(crate) fn build(cargo: &OsStr, tree: &Path, target_directory: &Path) -> Command {
    let mut command = crate::child_cargo::command(cargo);
    command
        .current_dir(tree)
        .env("CARGO_TARGET_DIR", target_directory)
        .args([
            "run",
            "--locked",
            "--package",
            "xtask",
            "--",
            "console",
            "build",
        ]);
    command
}
