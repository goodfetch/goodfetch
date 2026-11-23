//! `goodfetch-dataset` through its public interface: the Exports
//! `write_export` writes of a run file, as `run.export` writes them.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod sheet_rows;
