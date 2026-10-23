//! Collie's executable, driven as its Clients drive it (spec #213's Seam 1),
//! each test with its own Goodfetch home and a cleared environment, the
//! executable found through `CARGO_BIN_EXE_goodfetch-collie` (ADRs 0059, 0062
//! and 0094).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod attached;
mod audit;
#[cfg(unix)]
mod background_traffic;
#[cfg(unix)]
mod browser_pool;
mod browsers;
mod certs;
#[cfg(target_os = "macos")]
mod checkpoint;
mod clean_up_rules;
mod client_conformance;
mod commands;
mod config;
mod console;
mod content_origin;
mod daemon;
mod daemon_log;
mod daemon_run;
mod engine_choice;
mod events;
mod exit_codes;
#[cfg(target_os = "linux")]
mod governor;
mod help;
mod human_output;
mod invalid_calls;
mod journeys;
mod lasting_sign_ins;
#[cfg(unix)]
mod launch;
mod layout;
#[cfg(target_os = "linux")]
mod lazyfs;
mod locator;
mod log;
mod navigation_egress;
mod ops;
mod page;
#[cfg(unix)]
mod page_ends;
mod page_events;
mod parent_death;
mod private_endpoint;
mod proxy;
mod run;
mod secrets;
mod service_manager;
mod sessions;
mod settings_flags;
#[cfg(unix)]
mod shedding;
mod sign_in;
#[cfg(target_os = "linux")]
mod storage;
#[cfg(target_os = "linux")]
mod tmpfs;
mod upgrade_files;
mod usage;
mod views;
#[cfg(target_os = "linux")]
mod virtual_display;
mod websocket;
mod writer_lock;
