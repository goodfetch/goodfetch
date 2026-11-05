//! The Kennel's modules against real processes, sockets and files: the
//! Client against `goodfetch-scripted-engine` (ADR 0069), which the root
//! workspace builds and the test tool manifest names, never as a linked
//! crate; the settings file; the installed Product's `daemon start`; the
//! details window's process; and the Run controls against a real Daemon,
//! Collie's executable, built the same way.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod client;
mod daemon_start;
mod login;
mod real_daemon;
mod settings;
mod support;
mod window;
