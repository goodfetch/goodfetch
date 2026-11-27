//! The rule lists removal reads a page with, built once per version and
//! per host rather than once per page (spec #2164, issue 2186).
//!
//! **The generic set** ([`Generic`]) is built once per loaded
//! [`FilterLists`] and shared by every Run on them; what a host adds is
//! read against it.
//!
//! **The host cache** ([`HostCache`]), one per Run, keeps for each host,
//! combination of lists and per-list `generichide` bit what the host adds:
//! its own selectors less the generic set's, its procedural rules parsed
//! and compiled once, and the generic selectors its exceptions take back.
//! Its bound is `filter_lists.host_cache`, 0 building one per page, with
//! `moka`'s `TinyLFU` eviction.
//!
//! The `generichide` bit is the only part of a page's rules its whole URL
//! sets: `adblock` asks it of each list's `$generichide` exceptions, which
//! [`FilterLists`] keeps in a small engine per list, so the key costs no
//! more than the bit. The generic rules a class or ID keys stay per page,
//! as they depend on the classes and IDs it holds.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use adblock::cosmetic_filter_cache::UrlSpecificResources;
use adblock::request::Request;
use dom_query::Matcher;

use super::{Procedural, Rule, procedural};
use crate::cosmetic::{self, HIDDEN};
use crate::generic::Generic;
use crate::index::Index;
use crate::list::{List, ListSet};
use crate::rules::FilterLists;

/// One Run's rule lists by host: at most its bound of them, none kept at a
/// bound of 0.
pub struct HostCache {
    hosts: Option<moka::sync::Cache<HostKey, Arc<Host>>>,
}

/// What one host's rule list is built for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct HostKey {
    /// The digest of the rules' version.
    version: String,
    /// The page's host, as `adblock` reads it from its URL; none for a URL
    /// it does not read.
    host: Option<String>,
    /// The lists that are on.
    on: ListSet,
    /// The lists among them a `$generichide` exception turns the live
    /// generic rules off for, on the page's URL.
    generic_hide: ListSet,
    /// The lists among them a `$generichide` exception turns the
    /// removal-only cookie-section generic rules off for, on the page's URL.
    removal_generic_hide: ListSet,
}

impl HostKey {
    /// The key of the page at `url`, read with `lists` of the version
    /// `version`, the lists `on` on.
    pub(super) fn of(lists: &FilterLists, version: &str, url: &str, on: ListSet) -> Self {
        let host = Request::new(url, url, "document", "get")
            .ok()
            .map(|request| request.hostname);
        let generic_hide = on
            .lists()
            .filter(|list| host.is_some() && lists.generic_hide(*list, url))
            .fold(ListSet::default(), ListSet::with);
        let removal_generic_hide = on
            .lists()
            .filter(|list| host.is_some() && lists.removal_generic_hide(*list, url))
            .fold(ListSet::default(), ListSet::with);
        Self {
            version: version.to_owned(),
            host,
            on,
            generic_hide,
            removal_generic_hide,
        }
    }

    /// The lists whose live generic rules a class or ID keys apply.
    fn class_and_id(&self) -> ListSet {
        self.on.without(self.generic_hide)
    }

    /// The lists whose removal-only generic rules a class or ID keys apply.
    fn removal_class_and_id(&self) -> ListSet {
        self.on.without(self.removal_generic_hide)
    }

    /// The lists whose live generic rules no class or ID keys apply: none
    /// for a URL `adblock` does not read, which it gives no rules.
    fn generic(&self) -> ListSet {
        if self.host.is_some() {
            self.class_and_id()
        } else {
            ListSet::default()
        }
    }

    /// The lists whose removal-only generic rules no class or ID keys apply.
    fn removal_generic(&self) -> ListSet {
        if self.host.is_some() {
            self.removal_class_and_id()
        } else {
            ListSet::default()
        }
    }
}

/// What one host adds to the generic set, for one key.
pub(super) struct Host {
    /// The lists whose live generic set applies.
    pub(super) generic: ListSet,
    /// The lists whose removal-only cookie-section generic set applies.
    pub(super) removal_generic: ListSet,
    /// The host's own plain rules, in rule order, filed in `index`.
    pub(super) rules: Vec<HostRule>,
    pub(super) index: Index,
    /// The lists among `generic` that take each generic selector back, by
    /// its position in the live generic set.
    pub(super) excepted: HashMap<usize, ListSet>,
    /// The lists among `removal_generic` that take each removal-only
    /// generic selector back, by its position in the removal-only generic
    /// set.
    pub(super) removal_excepted: HashMap<usize, ListSet>,
    /// The merged exceptions of each list whose class and ID rules apply,
    /// paired with whether live and removal-only class/ID rules apply.
    pub(super) exceptions: Vec<(List, bool, bool, HashSet<String>)>,
    /// The procedural rules that are no plain CSS, in list order.
    pub(super) procedural: Vec<Rule<Procedural>>,
    /// The bytes it holds, about.
    bytes: u64,
}

/// One of a host's plain rules, ranked as `PageRules::css_rules` adds them
/// within a list: its hiding selectors (0), then its procedural rules that
/// are plain CSS (2); the page's class and ID rules come between (1).
pub(super) struct HostRule {
    pub(super) selector: String,
    pub(super) list: List,
    pub(super) rank: u8,
    /// Its compiled form: none for a rule that hides by other declarations,
    /// which removal leaves out, or one that does not compile.
    pub(super) matcher: Option<Matcher>,
}

impl HostCache {
    /// A cache of at most `bound` hosts' rule lists; at 0, each page builds
    /// its own.
    pub fn new(bound: u64) -> Self {
        Self {
            hosts: (bound > 0).then(|| moka::sync::Cache::new(bound)),
        }
    }

    /// Drops every host's rule list, as the Resource governor's `critical`
    /// level asks.
    pub fn clear(&self) {
        if let Some(hosts) = &self.hosts {
            hosts.invalidate_all();
            hosts.run_pending_tasks();
        }
    }

    /// How many hosts' rule lists it holds.
    pub fn entries(&self) -> u64 {
        self.hosts.as_ref().map_or(0, |hosts| {
            hosts.run_pending_tasks();
            hosts.entry_count()
        })
    }

    /// About how many bytes its rule lists hold, their keys included: a
    /// compiled selector, which the engine's cache of compilations shares,
    /// counts its handle only.
    pub fn bytes(&self) -> u64 {
        self.hosts.as_ref().map_or(0, |hosts| {
            hosts.run_pending_tasks();
            hosts
                .iter()
                .map(|(key, host)| {
                    let key = size_of::<HostKey>()
                        + key.version.capacity()
                        + key.host.as_ref().map_or(0, String::capacity);
                    key as u64 + host.bytes
                })
                .sum()
        })
    }

    /// The rule list of `key`, built by `build` unless the cache holds it.
    pub(super) fn host(&self, key: &HostKey, build: impl FnOnce(&HostKey) -> Host) -> Arc<Host> {
        match &self.hosts {
            Some(hosts) => hosts.get_with_by_ref(key, || Arc::new(build(key))),
            None => Arc::new(build(key)),
        }
    }
}

impl Host {
    /// What the page at `url` adds to `generic`, by the lists of `lists`
    /// `key` names, its selectors compiled through `compile`.
    pub(super) fn build(
        lists: &FilterLists,
        generic: &Generic,
        removal_generic: &Generic,
        url: &str,
        key: &HostKey,
        compile: &impl Fn(&str) -> Option<Matcher>,
    ) -> Self {
        let generic_lists = key.generic();
        let removal_generic_lists = key.removal_generic();
        let mut collected = Collected::default();
        let mut excepted: HashMap<usize, ListSet> = HashMap::new();
        let mut removal_excepted: HashMap<usize, ListSet> = HashMap::new();
        let mut exceptions = Vec::new();
        for (list, engine) in lists.engines() {
            if !key.on.contains(list) {
                continue;
            }
            let resources = engine.url_cosmetic_resources(url);
            let removal_resources = lists
                .removal_only(list)
                .map(|extra| extra.url_cosmetic_resources(url));
            let mut merged_exceptions = resources.exceptions.clone();
            if let Some(extra) = &removal_resources {
                merged_exceptions.extend(extra.exceptions.iter().cloned());
            }
            let live_applies = generic_lists.contains(list);
            let rem_applies = removal_generic_lists.contains(list);
            collected.add(
                &resources,
                list,
                generic,
                live_applies,
                &merged_exceptions,
                compile,
            );
            if let Some(extra) = &removal_resources {
                collected.add(
                    extra,
                    list,
                    removal_generic,
                    rem_applies,
                    &merged_exceptions,
                    compile,
                );
            }
            if live_applies {
                record_excepted(&merged_exceptions, generic, list, &mut excepted);
            }
            if rem_applies {
                record_excepted(
                    &merged_exceptions,
                    removal_generic,
                    list,
                    &mut removal_excepted,
                );
            }
            let live_cid = key.class_and_id().contains(list);
            let rem_cid = removal_resources.is_some() && key.removal_class_and_id().contains(list);
            if live_cid || rem_cid {
                exceptions.push((list, live_cid, rem_cid, merged_exceptions));
            }
        }
        collected
            .rules
            .sort_unstable_by(|a, b| (&a.0, a.1, a.2).cmp(&(&b.0, b.1, b.2)));
        let mut previous: Option<String> = None;
        let rules: Vec<HostRule> = collected
            .rules
            .into_iter()
            .map(|(selector, list, rank, hidden)| {
                // Only the first rule of a selector is ever applied.
                let first = previous.as_deref() != Some(selector.as_str());
                previous = Some(selector.clone());
                HostRule {
                    matcher: (first && hidden).then(|| compile(&selector)).flatten(),
                    selector,
                    list,
                    rank,
                }
            })
            .collect();
        let index = Index::of(rules.iter().map(|rule| rule.selector.as_str()));
        let mut host = Self {
            generic: generic_lists,
            removal_generic: removal_generic_lists,
            rules,
            index,
            excepted,
            removal_excepted,
            exceptions,
            procedural: collected.procedurals,
            bytes: 0,
        };
        host.bytes = host.held();
        host
    }

    /// The generic set's lists that apply at `at` here.
    pub(super) fn generic_lists(&self, at: usize, lists: ListSet) -> ListSet {
        lists
            .and(self.generic)
            .without(self.excepted.get(&at).copied().unwrap_or_default())
    }

    /// The first list a generic selector held by `lists` is credited to
    /// here, when any applies.
    pub(super) fn generic_list(&self, at: usize, lists: ListSet) -> Option<List> {
        self.generic_lists(at, lists).first()
    }

    /// The first list a removal-only generic selector held by `lists` is
    /// credited to here, when any applies.
    pub(super) fn removal_generic_list(&self, at: usize, lists: ListSet) -> Option<List> {
        lists
            .and(self.removal_generic)
            .without(self.removal_excepted.get(&at).copied().unwrap_or_default())
            .first()
    }

    /// About how many bytes it holds.
    fn held(&self) -> u64 {
        let rules: usize = self
            .rules
            .iter()
            .map(|rule| size_of::<HostRule>() + rule.selector.capacity())
            .sum();
        let excepted = (self.excepted.capacity() + self.removal_excepted.capacity())
            * size_of::<(usize, ListSet)>();
        let exceptions: usize = self
            .exceptions
            .iter()
            .map(|(_, _, _, exceptions)| {
                size_of::<(List, bool, bool, HashSet<String>)>()
                    + exceptions.capacity() * size_of::<String>()
                    + exceptions.iter().map(String::capacity).sum::<usize>()
            })
            .sum();
        let procedural: usize = self
            .procedural
            .iter()
            .map(|rule| size_of::<Rule<Procedural>>() + rule.name.capacity() + rule.kind.bytes())
            .sum();
        (size_of::<Self>() + rules + self.index.bytes() + excepted + exceptions + procedural) as u64
    }
}

/// Non-generic plain rules and procedural rules collected while building a
/// [`Host`].
#[derive(Default)]
struct Collected {
    rules: Vec<(String, List, u8, bool)>,
    procedurals: Vec<Rule<Procedural>>,
}

impl Collected {
    /// Collects the host's non-generic plain rules and procedural rules from
    /// `resources` for `list`, skipping any selector listed in `exceptions`.
    fn add(
        &mut self,
        resources: &UrlSpecificResources,
        list: List,
        generic: &Generic,
        applies: bool,
        exceptions: &HashSet<String>,
        compile: &impl Fn(&str) -> Option<Matcher>,
    ) {
        #[expect(
            clippy::iter_over_hash_type,
            reason = "the rules are sorted once read, so the order read is not the order kept"
        )]
        for selector in &resources.hide_selectors {
            if exceptions.contains(selector) {
                continue;
            }
            let generic_rule = generic
                .position(selector)
                .is_some_and(|at| generic.rule(at).1.contains(list));
            if !(applies && generic_rule) {
                self.rules.push((selector.clone(), list, 0, true));
            }
        }
        for filter in cosmetic::procedural(resources) {
            match filter.as_css() {
                Some((selector, style)) => {
                    let hidden = style == HIDDEN;
                    self.rules.push((selector, list, 2, hidden));
                }
                None => self.procedurals.extend(procedural(&filter, list, compile)),
            }
        }
    }
}

/// Records every generic selector in `generic` that `exceptions` takes back
/// for `list`.
fn record_excepted(
    exceptions: &HashSet<String>,
    generic: &Generic,
    list: List,
    excepted: &mut HashMap<usize, ListSet>,
) {
    #[expect(
        clippy::iter_over_hash_type,
        reason = "each exception goes into a map by position, so the order read is not the order kept"
    )]
    for exception in exceptions {
        if let Some(at) = generic.position(exception)
            && generic.rule(at).1.contains(list)
        {
            let lists = excepted.entry(at).or_default();
            *lists = lists.with(list);
        }
    }
}
