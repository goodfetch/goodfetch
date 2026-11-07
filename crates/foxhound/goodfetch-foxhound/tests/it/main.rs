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
mod body_limits;
mod bounds;
mod browser;
mod browser_fetch;
mod certificates;
mod client_conformance;
mod code_free;
mod code_free_resume;
mod command_line;
mod completeness;
mod content_types;
mod crawl;
mod crawl_command_line;
mod crawl_commands;
// The crawl benchmark's scenario, which `in_memory_crawl` checks.
#[path = "../../benches/scenarios.rs"]
#[expect(
    clippy::disallowed_methods,
    reason = "the scenario runs on the test Clock and Entropy (ADR 0059)"
)]
mod crawl_scenario;
// The run-state rows' plants, which `in_memory_crawl` proves.
#[path = "../../benches/plants.rs"]
mod crawl_plants;
mod daemon;
mod dataset_pages;
mod datasets;
mod decoding;
mod discovered_api;
mod enqueue_links;
mod escalation;
mod export_formats;
mod exports;
mod extra_loopback;
mod fetch_tiers;
mod frontier;
mod governor;
mod in_memory_crawl;
mod journeys;
mod kept_responses;
mod kept_tree;
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
mod soak;
mod sql_tab;
mod staging;
mod tier_race;
mod trap_links;
mod url_fragments;
mod use_signals;
mod waiting;
mod workers;
mod writer_lock;

/// Whether the Run warning `code` is the one a Run carries while it holds
/// to Clean-up rules older than 14 days: every Run that holds to the
/// embedded part carries it from 14 days after the part's date on, and a
/// test of the executable runs on the real calendar, which no build lets
/// it set (TESTING.md › Time). A test that is not about the Clean-up rules
/// compares the other warnings, so its outcome never depends on the date
/// it runs on; Collie's `clean_up_rules` tests prove the rule itself.
fn on_the_calendar(code: &str) -> bool {
    code == goodfetch_protocol::run::warning::CLEAN_UP_RULES_STALE
}
