//! Collie's executable, driven as its Clients drive it (spec #213's Seam 1),
//! each test with its own Goodfetch home and a cleared environment, the
//! executable found through `CARGO_BIN_EXE_goodfetch-collie` (ADRs 0059, 0062
//! and 0094).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod attached;
mod browsers;
mod config;
mod daemon;
mod events;
mod journeys;
mod log;
mod run;
