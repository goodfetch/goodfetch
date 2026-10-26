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
mod bounds;
mod browser_fetch;
mod completeness;
mod content_types;
mod crawl;
mod daemon;
mod datasets;
mod decoding;
mod enqueue_links;
mod exports;
mod fetch_tiers;
mod frontier;
mod governor;
mod journeys;
mod kept_responses;
mod label_pools;
mod learned_rules;
mod limits;
mod navigation_gate;
mod operator_policy;
mod outcomes;
mod pacing;
mod path_budgets;
mod payload;
mod politeness;
mod provenance;
mod records;
mod redundant_requests;
mod rejections;
mod robots_txt;
mod scope;
mod seeds;
mod served_calls;
mod service_manager;
mod session_pool;
mod sitemap;
mod staging;
mod tier_race;
mod tmpfs;
mod url_fragments;
mod use_signals;
mod workers;
mod writer_lock;
