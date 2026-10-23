//! Collie's pages, Sessions and the browsers behind them (spec #577): the
//! driver, which drives Chromium-family browsers over their debugging pipe
//! through the CDP client, the Session-level driver every browser behind a
//! Session shares (ADR 0129), and the page API's operation handlers, which
//! the composition value [`contribution`] hands to the engine (ADR 0087),
//! and the pure functions both Products' code calls: the engine's one URL
//! matcher ([`url_pattern`]) and robots.txt's group selection
//! ([`robots_txt`]). Its own crate so that Foxhound's crates fill its
//! interfaces without reaching its internals (STYLE.md, reason 2); it never
//! names a Foxhound type.

mod browser_fetch;
mod clean_up_versions;
mod consent;
mod driver;
mod errors;
mod guard;
mod hiding;
mod interception;
mod launch;
mod page_events;
mod pages;
mod pool;
mod proxy;
mod proxy_provider;
pub mod robots_txt;
mod sessions;
mod trace;
pub mod url_pattern;

pub use clean_up_versions::manifest_file;
pub use pages::contribution;
