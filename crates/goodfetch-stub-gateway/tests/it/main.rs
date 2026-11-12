//! The Stub gateway's integration tests (ADRs 0068 and 0096): its
//! executable, started with both Products' executables of the same
//! commit, reached as a Client reaches it.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod cloud_readiness;
mod gateway;
