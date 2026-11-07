//! The Cargo that `xtask` starts to build or run the workspace's targets.
//! `cargo run` gives `xtask` its package's variables (`CARGO_MANIFEST_DIR`,
//! `CARGO_PKG_NAME`, …), and a Cargo that inherits them sees each build
//! script that names one in `rerun-if-env-changed`, as `ring`'s does, as
//! changed: it reruns the script and rebuilds every crate above it, which
//! the next Cargo `just` starts, without them, rebuilds again. Every Cargo
//! `xtask` starts therefore starts here, a build's and a query's alike, and
//! runs without them.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::process::Command;

/// The variables `cargo run` sets for the program it runs, but `CARGO`,
/// which names Cargo itself (The Cargo Book, "Environment variables Cargo
/// sets for crates").
const RUN_VARIABLES: [&str; 23] = [
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_MANIFEST_LINKS",
    "CARGO_PKG_VERSION",
    "CARGO_PKG_VERSION_MAJOR",
    "CARGO_PKG_VERSION_MINOR",
    "CARGO_PKG_VERSION_PATCH",
    "CARGO_PKG_VERSION_PRE",
    "CARGO_PKG_AUTHORS",
    "CARGO_PKG_NAME",
    "CARGO_PKG_DESCRIPTION",
    "CARGO_PKG_HOMEPAGE",
    "CARGO_PKG_REPOSITORY",
    "CARGO_PKG_LICENSE",
    "CARGO_PKG_LICENSE_FILE",
    "CARGO_PKG_RUST_VERSION",
    "CARGO_PKG_README",
    "CARGO_CRATE_NAME",
    "CARGO_BIN_NAME",
    "CARGO_PRIMARY_PACKAGE",
    "CARGO_TARGET_TMPDIR",
    "CARGO_RUSTC_CURRENT_DIR",
    "OUT_DIR",
];

/// `cargo` as a command without the variables of the package `cargo run`
/// started `xtask` from.
pub(crate) fn command(cargo: &OsStr) -> Command {
    let mut command = Command::new(cargo);
    for name in RUN_VARIABLES {
        command.env_remove(name);
    }
    command
}
