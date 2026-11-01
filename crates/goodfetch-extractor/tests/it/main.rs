//! `goodfetch-extractor` through its public interface, in a process of its
//! own: what reading a page costs whatever it holds, past what the unit
//! tests' Quick-suite time allows (TESTING.md, Levels and layout).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod microdata_budget;
