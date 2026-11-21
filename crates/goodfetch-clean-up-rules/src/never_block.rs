//! The never-block list (ADR 0047): Goodfetch's own entries for challenge
//! and CAPTCHA providers, anti-bot sensors, anti-fraud and payment scripts,
//! and Consent prompts' scripts and messages, whose requests load over every
//! Filter list and over `browser.block_resources`. Read from `rules/never-block.toml` at build
//! time and compiled into a small `adblock` engine of its own when first
//! used, so no exception or `$important` rule of any Filter list reaches it.

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};

include!(concat!(env!("OUT_DIR"), "/never_block.rs"));

/// The never-block list, compiled.
pub struct NeverBlock {
    engine: Engine,
}

impl NeverBlock {
    /// Compiles the never-block list: a few dozen rules, in well under a
    /// millisecond.
    pub fn new() -> Self {
        let rules: String = RULES.iter().flat_map(|rule| [*rule, "\n"]).collect();
        let mut set = FilterSet::new(false);
        set.add_filter_list(rules, ParseOptions::default());
        Self {
            engine: Engine::new_with_filter_set(set),
        }
    }

    /// Whether an entry names `request`.
    pub(crate) fn names(&self, request: &adblock::request::Request) -> bool {
        self.engine.check_network_request(request).filter.is_some()
    }
}

impl Default for NeverBlock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
