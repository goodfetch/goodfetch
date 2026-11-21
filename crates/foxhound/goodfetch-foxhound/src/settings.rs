//! The settings a program's code gives the scopes it opens (ADRs 0083 and
//! 0093): Collie's [`Settings`] and keys, and Foxhound's own beside them,
//! a crawl's and a Label's among them, in the same modules, which mirror a
//! key's dotted name:
//!
//! ```
//! use goodfetch_foxhound::settings::{Settings, fetch, timeout};
//! use goodfetch_protocol::catalog::SettingScope;
//!
//! let label = Settings::new()
//!     .set(fetch::TIER, fetch::Tier::Browser)
//!     .set(timeout::HOOK, std::time::Duration::from_secs(5));
//! assert!(label.within(SettingScope::Label).is_ok());
//! ```
//!
//! A key a scope does not set, as `politeness::domain::CONCURRENCY` in a
//! Label's settings, fails with `setting_not_applicable` before anything
//! is sent, as Collie's keys do.

pub use goodfetch_collie_client::settings::*;

include!("generated/settings.rs");

#[cfg(test)]
mod tests;
