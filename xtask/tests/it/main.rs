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

mod advisories;
mod affected;
mod all_green;
mod allure;
mod axe_exceptions;
mod budgets;
mod clients;
mod codegen;
mod commit_messages;
mod compare_spider;
mod compat;
mod console;
mod containment;
#[cfg(unix)]
mod counts;
mod dco;
mod dependency_direction;
mod diagnostic;
mod egress;
mod egress_canary;
mod egress_hosts;
mod egress_rules;
mod evict_caches;
mod exercise_audit;
mod feature_generators;
mod features;
mod features_report;
mod fixture_size;
mod floor_guard;
mod gate;
mod gate_backstop;
mod head_order;
mod image_pins;
mod ingest;
mod journey_selection;
mod journey_variants;
mod journeys;
mod journeys_report;
mod junit_summary;
mod lazyfs_build;
mod merge_cap;
mod minute_ceiling;
mod mocking_lockfile;
mod modules;
mod no_retry;
mod notices;
mod order_seed;
mod os_services;
mod output_scans;
mod package_dependencies;
mod perf;
mod perf_build;
mod perf_ceilings;
mod perf_drift;
#[cfg(unix)]
mod perf_reference;
#[cfg(unix)]
mod perf_run;
mod pre_merge;
mod quarantine;
mod quick;
mod received_requests;
mod refusals;
mod register;
mod register_coverage;
mod release_suite;
mod repeat;
mod rss;
mod scan;
mod screenshots;
mod self_hosted;
mod size;
mod stress;
mod sweep_target;
mod targets;
mod telemetry;
mod test_tools;
mod tlc;
mod watchlist;
mod wording;
