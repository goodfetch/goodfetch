//! Collie's pages, Sessions and the browsers behind them (spec #577): the
//! driver, which drives Chromium-family browsers over their debugging pipe
//! through the CDP client, the Session-level driver every browser behind a
//! Session shares (ADR 0129), and the page API's operation handlers, which
//! the composition value [`contribution`] hands to the engine (ADR 0087).
//! Its own crate so that Foxhound's crates fill its interfaces without
//! reaching its internals (STYLE.md, reason 2); it never names a Foxhound
//! type.

mod driver;
mod errors;
mod guard;
mod launch;
mod pages;
mod trace;

pub use pages::contribution;
