//! `goodfetch-engine`'s integration tests: its public interface with real
//! local stand-ins, such as a name server on loopback.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod dns;
mod name_server;
