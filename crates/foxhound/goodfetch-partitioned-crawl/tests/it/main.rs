//! Integration and Simulation tests for `goodfetch-partitioned-crawl`
//! (ADRs 0054, 0060 and 0087).
#![cfg_attr(
    tokio_unstable,
    expect(
        clippy::doc_markdown,
        reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
    )
)]
#![cfg_attr(
    tokio_unstable,
    expect(
        clippy::unwrap_used,
        reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
    )
)]

// The Simulation's scenarios (ADR 0060), built only under `--cfg
// tokio_unstable` by `just simulate` and `just simulation`.
#[cfg(tokio_unstable)]
mod simulation;
