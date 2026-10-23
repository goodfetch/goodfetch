//! The never-block list (ADR 0047): Goodfetch's own entries for challenge
//! and CAPTCHA providers, anti-bot sensors, anti-fraud and payment scripts,
//! and Consent prompts' scripts and messages, whose requests load over every
//! Filter list and over `browser.block_resources`. Read from `rules/never-block.toml` at build
//! time and compiled into a small `adblock` engine of its own when first
//! used, so no exception or `$important` rule of any Filter list reaches it.

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};

/// What a never-block entry keeps loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// A challenge or CAPTCHA provider.
    Challenge,
    /// An anti-bot sensor, or the endpoint it reports to.
    AntiBot,
    /// An anti-fraud script.
    AntiFraud,
    /// A payment script.
    Payment,
    /// A Consent prompt's script or message, which Consent prompt rules
    /// need to see (ADR 0048).
    Consent,
}

/// One entry of the never-block list.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The vendor it is for.
    pub vendor: &'static str,
    /// What it keeps loading.
    pub kind: Kind,
    /// Its network rule, in Adblock Plus syntax.
    pub rule: &'static str,
    /// A request it keeps loading.
    pub example: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/never_block.rs"));

/// Every entry of the never-block list.
pub fn entries() -> &'static [Entry] {
    ENTRIES
}

/// The never-block list, compiled.
pub struct NeverBlock {
    engine: Engine,
}

impl NeverBlock {
    /// Compiles the never-block list: a few dozen rules, in well under a
    /// millisecond.
    pub fn new() -> Self {
        let rules: String = ENTRIES
            .iter()
            .flat_map(|entry| [entry.rule, "\n"])
            .collect();
        let mut set = FilterSet::new(false);
        set.add_filter_list(rules, ParseOptions::default());
        Self {
            engine: Engine::new_with_filter_set(set),
        }
    }

    /// No entry at all: what the Filter lists alone would do, which the
    /// tests compare the list with.
    pub fn empty() -> Self {
        Self {
            engine: Engine::default(),
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
