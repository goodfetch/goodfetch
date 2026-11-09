//! The engine's one bounded cache of compiled selectors, regular expressions
//! and globs (ADRs 0138 and 0140): each is compiled through its counted
//! function the first time its text is asked for and kept, so a Run's pages,
//! and the engine's later Runs, compile none of what an earlier page did. A
//! selector of the Clean-up rules is keyed by its text and the digest of the
//! rules' version too, as a Run holds to one version (ADR 0047). A lookup
//! takes no lock around a page's other lookups: the cache is concurrent.
//!
//! It holds at most [`CAPACITY`] compilations. One over the bound evicts one
//! kept before, the one least likely to be asked for again (moka's `TinyLFU`),
//! rather than grows: an evicted text asked for again is compiled again, and
//! counted again.

use std::sync::Arc;

use moka::Equivalent;

/// The most selectors the cache keeps: well above the 613 B1's default
/// Filter lists compile for a Run, and the few dozen the engine's own readers
/// and a Run's fields and handlers hold. A selector takes about a kilobyte,
/// so they stay under about 16 MB.
pub(crate) const CAPACITY: u64 = 16_384;

/// The most regular expressions, and apart the most globs, the cache keeps:
/// a Run's fields, Scope and Payload rules hold a few dozen.
const PATTERN_CAPACITY: u64 = 1_024;

/// A compilation's key: the Clean-up rules' digest for one of their
/// selectors, and its text.
#[derive(Debug, Hash, PartialEq, Eq)]
struct Key {
    rules: Option<Box<str>>,
    text: Box<str>,
}

/// A [`Key`] borrowed for a lookup, so a hit allocates nothing. Its derived
/// hash is the key's, field for field.
#[derive(Hash)]
struct Lookup<'a> {
    rules: Option<&'a str>,
    text: &'a str,
}

impl Equivalent<Key> for Lookup<'_> {
    fn equivalent(&self, key: &Key) -> bool {
        self.rules == key.rules.as_deref() && self.text == &*key.text
    }
}

impl Lookup<'_> {
    fn owned(&self) -> Key {
        Key {
            rules: self.rules.map(Box::from),
            text: Box::from(self.text),
        }
    }
}

/// One kind's part of the cache: what its counted function answered for
/// each text kept.
struct Part<V>(moka::sync::Cache<Key, V>);

impl<V: Clone + Send + Sync + 'static> Part<V> {
    fn bounded(capacity: u64) -> Self {
        Self(moka::sync::Cache::new(capacity))
    }

    /// What `compile` answers for `lookup`, compiled only when it is not
    /// kept, then kept. Two threads that miss the same text at once compile
    /// it once.
    fn kept(&self, lookup: &Lookup<'_>, compile: impl FnOnce() -> V) -> V {
        if let Some(kept) = self.0.get(lookup) {
            return kept;
        }
        self.0.get_with(lookup.owned(), compile)
    }

    /// How many it keeps now, its evictions done.
    #[cfg(test)]
    fn len(&self) -> u64 {
        self.0.run_pending_tasks();
        self.0.entry_count()
    }
}

/// The cache, shared by every clone of one engine's
/// [`CountedWork`](crate::CountedWork).
pub(crate) struct Compiled {
    selectors: Part<Option<dom_query::Matcher>>,
    regexes: Part<Result<regex::Regex, regex::Error>>,
    globs: Part<Result<glob::Pattern, Arc<glob::PatternError>>>,
}

impl Default for Compiled {
    fn default() -> Self {
        Self {
            selectors: Part::bounded(CAPACITY),
            regexes: Part::bounded(PATTERN_CAPACITY),
            globs: Part::bounded(PATTERN_CAPACITY),
        }
    }
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Compiled")
            .field("selectors", &self.selectors.0.entry_count())
            .finish_non_exhaustive()
    }
}

impl Compiled {
    /// The selector `text`, of the Clean-up rules of the digest `rules` when
    /// some, as `compile` answers it.
    pub(crate) fn selector(
        &self,
        rules: Option<&str>,
        text: &str,
        compile: impl FnOnce() -> Option<dom_query::Matcher>,
    ) -> Option<dom_query::Matcher> {
        self.selectors.kept(&Lookup { rules, text }, compile)
    }

    /// The regular expression `text`, as `compile` answers it.
    pub(crate) fn regex(
        &self,
        text: &str,
        compile: impl FnOnce() -> Result<regex::Regex, regex::Error>,
    ) -> Result<regex::Regex, regex::Error> {
        self.regexes.kept(&Lookup { rules: None, text }, compile)
    }

    /// The glob `text`, as `compile` answers it.
    pub(crate) fn glob(
        &self,
        text: &str,
        compile: impl FnOnce() -> Result<glob::Pattern, glob::PatternError>,
    ) -> Result<glob::Pattern, Arc<glob::PatternError>> {
        self.globs.kept(&Lookup { rules: None, text }, || {
            compile().map_err(Arc::new)
        })
    }

    /// How many selectors it keeps now, its evictions done.
    #[cfg(test)]
    pub(crate) fn selectors(&self) -> u64 {
        self.selectors.len()
    }
}
