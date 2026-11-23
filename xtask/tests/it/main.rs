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
mod api_check;
mod batch_check;
mod browser_releases;
mod budgets;
mod clients;
#[cfg(unix)]
mod clippy_excluded;
mod codegen;
mod commit_messages;
mod compare;
#[cfg(unix)]
mod compare_recipe;
mod compat;
#[cfg(unix)]
mod compiler_cache;
mod containment;
#[cfg(unix)]
mod counts;
mod dco;
mod deny_products;
mod dependency_direction;
mod diagnostic;
mod doc_obligations;
mod docs_samples;
mod egress;
mod egress_canary;
mod egress_hosts;
mod egress_rules;
mod evict_caches;
mod exercise_audit;
mod existing_code;
mod feature_generators;
mod features;
mod features_report;
mod fixture_size;
mod floor_guard;
mod fuzz_nightly;
mod gate;
mod gate_backstop;
mod gate_worktree;
mod head_order;
mod ignored_sources;
mod image_pins;
mod ingest;
mod journey_selection;
mod journey_variants;
mod journeys;
mod journeys_report;
mod junit_summary;
mod kennel_manifest;
mod lazyfs_build;
mod load_proofs;
mod mask_writers;
mod merge_cap;
mod merge_pr;
mod minute_ceiling;
mod mocking_lockfile;
mod modules;
mod mutants_typescript;
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
mod perf_script;
#[cfg(unix)]
mod pgo;
mod pre_merge;
mod quarantine;
mod quick;
mod received_requests;
mod refusals;
mod register;
mod register_coverage;
mod release;
mod release_suite;
mod repeat;
mod rss;
mod runtimes;
mod scan;
mod screenshots;
mod sdk_exports;
mod sdk_manifests;
mod size;
mod static_pie;
mod stress;
mod sweep_target;
mod targets;
mod telemetry;
mod test_health;
mod test_tools;
mod tlc;
mod watchlist;
mod wording;
mod workflow_policy;
mod workspace_hack;
