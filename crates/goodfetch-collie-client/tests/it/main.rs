//! `goodfetch-collie-client` through its public interface, against a
//! Collie engine with only the engine's own operations (ADR 0062): over
//! area 3's in-memory network, and over a Unix socket the test listens on.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

#[cfg(unix)]
mod attached;
mod closing;
mod connection;
mod connection_type;
#[cfg(unix)]
mod daemon;
mod duration;
mod error;
mod events;
mod file_saving;
mod forms;
mod handler_requests;
mod inputs;
mod limits;
mod logging;
mod navigations;
mod points_in_time;
mod results;
mod scripted_daemon;
mod session_events;
mod settings;
mod settings_types;
mod start;
