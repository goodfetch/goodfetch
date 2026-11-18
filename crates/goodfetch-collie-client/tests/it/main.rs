//! `goodfetch-collie-client` through its public interface, against a
//! Collie engine with only the engine's own operations (ADR 0062): over
//! area 3's in-memory network, and over a Unix socket the test listens on.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod connection;
#[cfg(unix)]
mod daemon;
mod forms;
