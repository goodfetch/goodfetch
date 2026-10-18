//! Seam A: `xtask`'s command line, run against fixture workspaces copied into
//! a temporary folder per test (spec #130, Testing Decisions).
#![expect(
    clippy::disallowed_methods,
    reason = "the harness copies fixtures with blocking std::fs and runs xtask as a process; it is no async crate"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod github;
mod workspace;

mod all_green;
mod dependency_direction;
mod fixture_size;
mod mocking_lockfile;
mod modules;
mod self_hosted;
