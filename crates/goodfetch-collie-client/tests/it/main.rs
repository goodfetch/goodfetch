//! `goodfetch-collie-client` through its public interface, against a
//! Collie engine with only the engine's own operations (ADR 0062): over
//! area 3's in-memory network, and over a Unix socket the test listens on.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

#[cfg(unix)]
mod attached;
mod call_cancelled;
mod cancellation;
mod cdp_url_errors;
mod closing;
mod compile_fail;
mod connection;
#[cfg(unix)]
mod daemon;
mod errors_and_outcomes;
mod events;
mod file_saving;
mod handler_requests;
mod limits;
mod logging;
mod navigations;
mod points_in_time;
mod product_and_provisioning;
mod results;
mod scripted_daemon;
mod session_events;
mod settings;
mod start;
mod web_socket;
