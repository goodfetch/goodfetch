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
mod axe_exceptions;
mod budgets;
mod codegen;
mod commit_messages;
mod compat;
mod containment;
mod dco;
mod dependency_direction;
mod diagnostic;
mod egress;
mod egress_canary;
mod egress_hosts;
mod egress_rules;
mod evict_caches;
mod feature_generators;
mod features;
mod fixture_size;
mod ingest;
mod journey_selection;
mod journey_variants;
mod journeys;
mod junit_summary;
mod minute_ceiling;
mod mocking_lockfile;
mod modules;
mod no_retry;
mod notices;
mod os_services;
mod package_dependencies;
mod perf;
mod perf_build;
mod perf_ceilings;
mod perf_drift;
mod perf_run;
mod quarantine;
mod release_suite;
mod repeat;
mod rss;
mod scan;
mod screenshots;
mod self_hosted;
mod size;
mod stress;
mod test_tools;
mod tlc;
mod wording;
