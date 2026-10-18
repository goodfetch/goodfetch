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
mod store;
mod workspace;

mod affected;
mod all_green;
mod allure;
mod codegen;
mod compat;
mod containment;
mod dco;
mod dependency_direction;
mod diagnostic;
mod egress;
mod egress_canary;
mod egress_hosts;
mod egress_rules;
mod fixture_size;
mod ingest;
mod minute_ceiling;
mod mocking_lockfile;
mod modules;
mod no_retry;
mod package_dependencies;
mod perf;
mod perf_drift;
mod quarantine;
mod release_suite;
mod repeat;
mod scan;
mod screenshots;
mod self_hosted;
mod size;
mod wording;
