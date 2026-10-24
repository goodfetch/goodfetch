//! Foxhound's executable, driven as its Clients drive it, for what only
//! Foxhound carries (ADR 0094); everything Collie carries is tested on
//! Collie's executable. The executable is found through
//! `CARGO_BIN_EXE_goodfetch-foxhound`.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod admission;
mod attached;
mod audit;
mod bounds;
mod crawl;
mod daemon;
mod frontier;
mod governor;
mod journeys;
mod learned_rules;
mod limits;
mod outcomes;
mod politeness;
mod scope;
mod seeds;
mod service_manager;
mod session_pool;
mod workers;
mod writer_lock;
