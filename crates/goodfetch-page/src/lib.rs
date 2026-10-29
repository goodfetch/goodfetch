//! Collie's pages, Sessions and the browsers behind them (spec #577): the
//! driver, which drives Chromium-family browsers over their debugging pipe
//! through the CDP client, the Session-level driver every browser behind a
//! Session shares (ADR 0129), and the page API's operation handlers, which
//! the composition value [`contribution`] hands to the engine (ADR 0087),
//! and the pure functions both Products' code calls: the engine's one URL
//! matcher ([`url_pattern`]) and robots.txt's group selection
//! ([`robots_txt`]), and the interfaces Foxhound fills on the pages it
//! hands to handlers ([`page_value`]), and the seam its Product's Block
//! classification fills ([`blocks`]), and the Run trace's Provenance log
//! ([`provenance`]), which Foxhound's fetch tiers also write. Its own crate so that Foxhound's crates fill its
//! interfaces without reaching its internals (STYLE.md, reason 2); it never
//! names a Foxhound type.

pub mod blocks;
mod borrowed;
mod browser_fetch;
mod clean_up_versions;
mod consent;
mod dialogs;
mod driver;
mod egress_relay;
mod errors;
mod guard;
mod hiding;
mod interception;
mod launch;
pub mod noindex;
mod page_events;
mod page_functions;
pub mod page_value;
mod pages;
mod paused_body;
mod pool;
mod proxy;
mod proxy_provider;
mod recovery;
pub mod robots_txt;
mod sessions;
mod trace;
pub mod url_pattern;

pub use clean_up_versions::manifest_file;
pub use pages::{contribution, contribution_with};
/// The proxy-or-site rule (spec #577): who answered a request sent through
/// a proxy, which Foxhound's fetch tiers apply to their own answers too
/// (spec #579).
pub use proxy::{Answer as ProxyAnswer, Party, party};
pub use trace::provenance;

/// Captures a browser build's ALPS payload when the build is pinned
/// (ADR 0042), for `pin-chrome-for-testing alps`.
pub use egress_relay::capture::capture_application_settings;

/// The egress relay's fuzz targets (ADR 0067), whose seeds and past
/// crashers `fuzz_replay!` replays on every OS.
pub use egress_relay::{
    fuzz_client_hello as fuzz_relay_client_hello, fuzz_request_head as fuzz_relay_request_head,
};
