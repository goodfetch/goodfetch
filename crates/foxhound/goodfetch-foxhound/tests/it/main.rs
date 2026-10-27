//! Foxhound's executable, driven as its Clients drive it, for what only
//! Foxhound carries (ADR 0094); everything Collie carries is tested on
//! Collie's executable. The executable is found through
//! `CARGO_BIN_EXE_goodfetch-foxhound`.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod addresses;
mod admission;
mod attached;
mod audit;
mod backoff_classes;
mod bounds;
mod browser;
mod browser_fetch;
mod certificates;
mod client_conformance;
mod code_free;
mod completeness;
mod content_types;
mod crawl;
mod crawl_commands;
// The crawl benchmark's scenario, which `in_memory_crawl` checks.
#[path = "../../benches/scenarios.rs"]
#[expect(
    clippy::disallowed_methods,
    reason = "the scenario runs on the test Clock and Entropy (ADR 0059)"
)]
mod crawl_scenario;
mod daemon;
mod dataset_pages;
mod datasets;
mod decoding;
mod discovered_api;
mod enqueue_links;
mod export_formats;
mod exports;
mod extra_loopback;
mod fetch_tiers;
mod frontier;
mod governor;
mod in_memory_crawl;
mod journeys;
mod kept_responses;
mod label_pools;
mod learned_rules;
mod light_browser;
mod limits;
mod navigation_gate;
mod operator_policy;
mod outcomes;
mod pacing;
mod path_budgets;
mod payload;
mod politeness;
mod politeness_records;
mod pool_proxies;
mod provenance;
mod records;
mod redundant_requests;
mod rejections;
mod retry_failed;
mod robots_txt;
mod sample_comparison;
mod scope;
mod seeds;
mod served_calls;
mod service_manager;
mod session_pool;
mod sitemap;
mod sql_tab;
mod staging;
mod tier_race;
mod tmpfs;
mod trap_links;
mod url_fragments;
mod use_signals;
mod workers;
mod writer_lock;
