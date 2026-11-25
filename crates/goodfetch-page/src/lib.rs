//! Collie's pages, Sessions and the browsers behind them (spec #577): the
//! driver, which drives Chromium-family browsers over their debugging pipe
//! through the CDP client, the Session-level driver every browser behind a
//! Session shares (ADR 0129), and the page API's operation handlers, which
//! the composition value [`contribution`] hands to the engine (ADR 0087),
//! and the pure functions both Products' code calls: the engine's one URL
//! matcher ([`url_pattern`]), robots.txt's group selection
//! ([`robots_txt`]) and the resolver of a page's main-content step
//! ([`extractor`]), and the interfaces Foxhound fills on the pages it
//! hands to handlers ([`page_value`]), and the seam its Product's Block
//! classification fills ([`blocks`]), and the Run trace's Provenance log
//! ([`provenance`]), which Foxhound's fetch tiers also write. Its own crate so that Foxhound's crates fill its
//! interfaces without reaching its internals (STYLE.md, reason 2); it never
//! names a Foxhound type.

mod access_mode;
pub mod blocks;
mod borrowed;
mod browser_fetch;
mod clean_up_versions;
mod consent;
mod dialogs;
pub mod document_request;
mod driver;
mod egress_relay;
mod errors;
pub mod extractor;
pub mod fields;
mod guard;
mod hiding;
mod interception;
pub mod kept_tree;
mod launch;
mod live_view;
pub mod noindex;
mod page_events;
mod page_functions;
pub mod page_value;
mod pages;
mod paused_body;
mod pool;
mod proxy;
mod proxy_provider;
mod proxy_status;
mod recovery;
pub mod robots_txt;
mod sessions;
mod trace;
pub mod url_pattern;
mod version_check;

pub use clean_up_versions::manifest_file;
pub use launch::{Launcher, Scripted, ScriptedBrowsers};
pub use pages::{contribution, contribution_with};
/// The proxy-or-site rule (spec #577): who answered a request sent through
/// a proxy, which Foxhound's fetch tiers apply to their own answers too
/// (spec #579).
pub use proxy::{Answer as ProxyAnswer, Party, party};
/// A Session's proxy as a fetch tier's own HTTP transport sends through
/// it (#1640), opened from the same settings and Proxy-provider driver as
/// a browser Session's.
pub use proxy::{ProxyRoute, TransportPermit, TransportProxy};
pub use trace::provenance;

/// What every relay of a browser's connections holds to, which Foxhound's
/// light-browser bridge is one of: its deadlines and its accept loop.
pub use egress_relay::deadlines as relay_deadlines;
pub use proxy::accepting as relay_accepting;

/// Captures a browser build's ALPS payload when the build is pinned
/// (ADR 0042), for `pin-chrome-for-testing alps`.
pub use egress_relay::capture::capture_application_settings;

/// The egress relay's fuzz targets (ADR 0067), whose seeds and past
/// crashers `fuzz_replay!` replays on every OS.
pub use egress_relay::{
    fuzz_client_hello as fuzz_relay_client_hello, fuzz_request_head as fuzz_relay_request_head,
};
