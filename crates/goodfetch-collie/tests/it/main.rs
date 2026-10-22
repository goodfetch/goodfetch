//! Collie's executable, driven as its Clients drive it (spec #213's Seam 1),
//! each test with its own Goodfetch home and a cleared environment, the
//! executable found through `CARGO_BIN_EXE_goodfetch-collie` (ADRs 0059, 0062
//! and 0094).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod attached;
mod browsers;
#[cfg(target_os = "macos")]
mod checkpoint;
mod client_conformance;
mod commands;
mod config;
mod console;
mod daemon;
mod daemon_run;
mod events;
mod help;
mod human_output;
mod invalid_calls;
mod journeys;
mod launch;
mod layout;
#[cfg(target_os = "linux")]
mod lazyfs;
mod locator;
mod log;
mod page;
mod parent_death;
mod private_endpoint;
mod run;
mod service_manager;
mod settings_flags;
mod upgrade_files;
mod usage;
#[cfg(target_os = "linux")]
mod virtual_display;
mod writer_lock;
