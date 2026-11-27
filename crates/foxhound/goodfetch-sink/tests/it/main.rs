//! The Sink conformance suite against the in-binary SQLite Sink (ADR
//! 0026): what a destination holds once a crawl's Records were delivered
//! through the delivery driver, under kills, lost answers and a zombie
//! copy of the Run, read from the destination's own rows.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod cloud_stores;
mod conformance;
mod dead_letters;
mod faults;
mod files;
mod model;
mod outlive;
mod postgres;
mod retries;
mod run_files;
mod schema_drift;
// The Simulation's scenarios (ADR 0060), built only under `--cfg
// tokio_unstable` by `just simulate` and `just simulation`.
#[cfg(tokio_unstable)]
mod simulation;
mod throughput;
mod webhook;
mod write_modes;
