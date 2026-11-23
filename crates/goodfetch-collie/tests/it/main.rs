//! Collie's executable, driven as its Clients drive it (spec #213's Seam 1),
//! each test with its own Goodfetch home and a cleared environment, the
//! executable found through `CARGO_BIN_EXE_goodfetch-collie` (ADRs 0059, 0062
//! and 0094).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

/// Collie's executable, which every test drives.
const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The `hello` these tests say to a Daemon, or to an Attached engine's
/// private endpoint: the TypeScript SDK's, needing Collie, with `token`.
fn daemon_hello(token: &str) -> goodfetch_test_client::Hello {
    goodfetch_test_client::Hello::needing(goodfetch_protocol::catalog::Product::Collie)
        .surface(goodfetch_protocol::connection::Surface::SdkTypescript)
        .token(token)
}

/// A Client of the Daemon `found` names, past that `hello`.
fn daemon_client(
    found: &goodfetch_test_support::RuntimeFile,
) -> goodfetch_test_client::DaemonClient {
    goodfetch_test_client::DaemonClient::opened(&found.endpoint, &daemon_hello(&found.token))
}

mod ai_work;
mod attached;
mod audit;
#[cfg(unix)]
mod background_traffic;
#[cfg(unix)]
mod borrowed_browser;
mod browser;
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
mod daemon;
mod daemon_commands;
mod daemon_log;
mod daemon_run;
mod doctor;
mod domain_narrowing;
mod egress_relay;
mod endpoint;
mod engine_choice;
mod events;
mod exit_codes;
mod extra_loopback;
#[cfg(target_os = "linux")]
mod governor;
mod help;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod huge_pages;
mod human_output;
#[cfg(unix)]
mod identified;
mod install_docs;
mod invalid_calls;
mod journeys;
#[cfg(unix)]
mod launch;
mod layout;
#[cfg(target_os = "linux")]
mod lazyfs;
mod locator;
mod log;
mod navigation_egress;
mod needs_you;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod no_console;
mod operations_guide;
mod ops;
mod other_product;
mod page;
#[cfg(unix)]
mod page_ends;
mod page_events;
mod parent_death;
mod plugin;
mod private_endpoint;
mod private_ranges;
mod proxy;
#[cfg(unix)]
mod proxy_limits;
mod relay_fingerprint;
mod request_kinds;
mod run;
mod rust_client;
mod sdk;
mod secrets;
mod security_controls;
mod service_manager;
mod sessions;
mod settings_flags;
#[cfg(unix)]
mod shedding;
mod signing_key;
mod skill_examples;
mod skill_install;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod source_build;
#[cfg(target_os = "linux")]
mod storage;
#[cfg(target_os = "linux")]
mod threads;
mod upgrade_files;
mod usage;
mod views;
#[cfg(target_os = "linux")]
mod virtual_display;
mod websocket;
mod writer_lock;
