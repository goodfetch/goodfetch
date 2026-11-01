//! `goodfetch-frontier` through its public interface, in a process of its
//! own: the Frontier at scale (ADR 0134).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod at_scale;
