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
mod borrowed_browser;
mod browser;
#[cfg(unix)]
mod browser_pool;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod browser_sandbox;
mod canaries;
mod certificate_exceptions;
mod certs;
#[cfg(target_os = "macos")]
mod checkpoint;
mod clean_up_rules;
mod client_conformance;
mod command_line;
mod commands;
mod config;
mod console;
mod console_life;
mod content_origin;
mod daemon;
mod daemon_commands;
mod daemon_log;
mod daemon_run;
mod domain_narrowing;
mod egress_relay;
mod endpoint;
mod engine_choice;
mod event_stream;
mod events;
mod exit_codes;
mod extra_loopback;
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
mod needs_you;
mod offline_key;
mod operations_guide;
mod ops;
mod other_product;
mod page;
#[cfg(unix)]
mod page_ends;
mod page_events;
mod parent_death;
mod private_endpoint;
mod private_ranges;
mod proxy;
mod proxy_limits;
mod relay_fingerprint;
mod request_kinds;
mod run;
mod secrets;
mod security_controls;
mod service_manager;
mod sessions;
mod settings_flags;
#[cfg(unix)]
mod shedding;
mod sign_in;
#[cfg(target_os = "linux")]
mod storage;
mod upgrade_files;
mod usage;
mod views;
#[cfg(target_os = "linux")]
mod virtual_display;
mod websocket;
mod writer_lock;
