//! The Sink conformance suite against the in-binary SQLite Sink (ADR
//! 0026): what a destination holds once a crawl's Records were delivered
//! through the delivery driver, under kills, lost answers and a zombie
//! copy of the Run, read from the destination's own rows.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod conformance;
mod faults;
mod run_files;
