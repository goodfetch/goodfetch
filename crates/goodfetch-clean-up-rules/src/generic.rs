//! The generic set (spec #2164, issue 2186): each list's generic hiding
//! selectors that no class or ID keys, read once per loaded version from a
//! reserved host no list names, and shared by every Run on it. Each
//! selector is kept with the lists that hold it and filed in one
//! [`Index`]; its compiled form is made on first use.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use adblock::Engine;
use dom_query::Matcher;

use crate::index::Index;
use crate::list::{List, ListSet};

/// The page the generic set is read for: a reserved host (RFC 6761) no
/// Filter list names, so its rules are the generic ones alone.
pub(crate) const GENERIC_PROBE: &str = "https://generic.goodfetch.invalid/";

/// The generic hiding selectors no class or ID keys, of every list, in
/// selector order, filed in one index.
pub(crate) struct Generic {
    rules: Vec<GenericRule>,
    index: Index,
}

/// One generic selector, the lists that hold it, and its compiled form,
/// made on first use; none when it does not compile.
struct GenericRule {
    selector: String,
    lists: ListSet,
    matcher: OnceLock<Option<Matcher>>,
}

impl Generic {
    /// The generic set of a version's `engines`, each list's: its hiding
    /// selectors for [`GENERIC_PROBE`].
    pub(crate) fn of(engines: [(List, &Engine); 3]) -> Self {
        let mut by_selector: BTreeMap<String, ListSet> = BTreeMap::new();
        for (list, engine) in engines {
            #[expect(
                clippy::iter_over_hash_type,
                reason = "each selector goes into an ordered map, so the order read is not the order kept"
            )]
            for selector in engine.url_cosmetic_resources(GENERIC_PROBE).hide_selectors {
                let held = by_selector.entry(selector).or_default();
                *held = held.with(list);
            }
        }
        let rules: Vec<GenericRule> = by_selector
            .into_iter()
            .map(|(selector, lists)| GenericRule {
                selector,
                lists,
                matcher: OnceLock::new(),
            })
            .collect();
        let index = Index::of(rules.iter().map(|rule| rule.selector.as_str()));
        Self { rules, index }
    }

    /// The position of `selector`, when the set holds it.
    pub(crate) fn position(&self, selector: &str) -> Option<usize> {
        self.rules
            .binary_search_by(|rule| rule.selector.as_str().cmp(selector))
            .ok()
    }

    /// The selector at `at`, and the lists that hold it.
    pub(crate) fn rule(&self, at: usize) -> (&str, ListSet) {
        let rule = &self.rules[at];
        (&rule.selector, rule.lists)
    }

    /// The compiled form of the selector at `at`, compiled through
    /// `compile` on first use.
    pub(crate) fn matcher(
        &self,
        at: usize,
        compile: impl FnOnce(&str) -> Option<Matcher>,
    ) -> Option<&Matcher> {
        let rule = &self.rules[at];
        rule.matcher
            .get_or_init(|| compile(&rule.selector))
            .as_ref()
    }

    pub(crate) fn index(&self) -> &Index {
        &self.index
    }
}
