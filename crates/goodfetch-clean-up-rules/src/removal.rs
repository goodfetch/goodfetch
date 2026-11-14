//! Extraction-side removal (ADR 0047): before dom-smoothie, htmd or any AI
//! input, the elements the cosmetic rules of the lists that are on match
//! go from a page's HTML, in every tier, inside its shadow roots too (a
//! page serialized with its shadow roots holds each, open or closed, as a
//! `<template shadowrootmode>`), so hidden text never reaches extracted
//! output. Procedural rules apply here where live hiding cannot: those
//! that select by CSS, by their text (`:has-text`, `:min-text-length`) and
//! by an ancestor (`:upward`); the rest are left out. The caller learns
//! what went, each element with its rule, its text and the links inside it,
//! which area 6's trap-link judgement leaves out (ADR 0035). The prompts
//! the Consent prompt runner hid go the same way, by the selectors of its
//! rules ([`remove_selected`], ADR 0048). A removal over a fifth of the
//! main content's text is flagged ([`Share`]).
//!
//! Removal takes the elements out of the page's one tree, in place, and
//! answers what it took; its caller serializes the tree only for what reads
//! the cleaned page as text (ADR 0138). Its selectors come from the engine's
//! bounded cache of compilations, keyed by their text and the digest of the
//! rules' version (`CountedWork::rules_selector`), so a page compiles none
//! that an earlier page, of this Run or another, compiled.
//!
//! An element is tested only against the plain rules it could match: the
//! page's rules are filed in an [`Index`] by a key of each one's subject,
//! and one walk tests each element against the rules its own ID, classes,
//! attributes and tag file, the earliest in rule order winning, as it did
//! when every element was tested against every rule (spec #2164).

mod hosts;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use adblock::cosmetic_filter_cache::ProceduralOrActionFilter;
use adblock::filters::cosmetic::{CosmeticFilterAction, CosmeticFilterOperator};
use dom_query::{Matcher, NodeId, NodeRef, Selection};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use selectors::context::SelectorCaches;

pub use self::hosts::HostCache;
use self::hosts::{Host, HostKey};
use crate::generic::Generic;
use crate::index::Index;
use crate::list::List;
use crate::matching::Choices;
use crate::rules::FilterLists;

/// The share of the main content's text above which a removal is flagged
/// in Provenance (ADR 0047's share, which spec #577 sets): a fifth.
pub const FLAGGED_SHARE: f64 = 0.20;

/// What removal took from a document.
#[derive(Clone, Debug, Default)]
pub struct Removal {
    /// Each element removed, in the order of its root (the document first,
    /// then each shadow root as the document holds it) and, within one, in
    /// document order, those of plain CSS rules before those of procedural
    /// ones.
    pub removed: Vec<Removed>,
    /// Their text, white space aside, in characters.
    pub removed_text: usize,
}

/// One element removal took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    /// The rule that matched it: its selector, or a procedural rule's text.
    pub rule: String,
    /// The list the rule comes from; none for a rule the Consent prompt
    /// runner hid a prompt with.
    pub list: Option<List>,
    /// Whether it sat in a shadow root.
    pub in_shadow_root: bool,
    /// Its text, white space aside, in characters.
    pub text: usize,
    /// The `href` of each link it held, itself included.
    pub links: Vec<String>,
}

/// The Clean-up rules a page is cleaned with: the Filter lists of one
/// version, that version's digest, what the page's settings turn on, and
/// the rule lists of the hosts its Run read pages of.
#[derive(Clone, Copy)]
pub struct Rules<'a> {
    /// The Filter lists.
    pub lists: &'a FilterLists,
    /// The digest of their version, which keys their compiled selectors.
    pub version: &'a str,
    /// Which of them are on.
    pub choices: &'a Choices,
    /// The Run's host cache.
    pub hosts: &'a HostCache,
}

/// One rule as removal applies it.
struct Rule<K> {
    name: String,
    list: List,
    kind: K,
}

/// A procedural rule's operators, in order.
struct Procedural(Vec<Step>);

impl Procedural {
    /// About how many bytes its operators hold beyond their own size: a
    /// compiled selector, which the engine's cache shares, none.
    fn bytes(&self) -> usize {
        let Self(steps) = self;
        steps.capacity() * size_of::<Step>()
            + steps
                .iter()
                .map(|step| match step {
                    Step::HasText(text) => text.capacity(),
                    Step::Css(_)
                    | Step::MinTextLength(_)
                    | Step::UpwardBy(_)
                    | Step::UpwardTo(_) => 0,
                })
                .sum::<usize>()
    }
}

/// One procedural operator removal evaluates.
enum Step {
    Css(Matcher),
    HasText(String),
    MinTextLength(usize),
    UpwardBy(usize),
    UpwardTo(Matcher),
}

/// Removes from `document`, the tree of the page at `url`, in place, every
/// element the cosmetic rules of `rules` match, their selectors compiled
/// through the engine's cache on `work`, and answers what it removed. It
/// counts on `work` each rule list it builds for a host its Run's host
/// cache does not hold (`F1.rule_list_builds`), and each rule it tests: a
/// plain rule against one element that could match it, a procedural one
/// over one root (`F1.rule_tests`).
pub fn remove(document: &ParsedHtml, url: &str, rules: Rules<'_>, work: &CountedWork) -> Removal {
    let on = rules.choices.lists();
    if on.is_empty() {
        return Removal::default();
    }
    let selector =
        |text: &str| work.rules_selector(Row::PageSelectorCompilations, rules.version, text);
    let generic = rules.lists.generic();
    let host = rules
        .hosts
        .host(&HostKey::of(rules.lists, rules.version, url, on), |key| {
            work.count(Row::PageRuleListBuilds);
            Host::build(rules.lists, generic, url, key, &selector)
        });
    let roots = roots(document, work);
    let page = class_and_id_rules(rules.lists, &host, &roots, &selector);
    if host.generic.is_empty()
        && host.rules.is_empty()
        && page.rules.is_empty()
        && host.procedural.is_empty()
    {
        return Removal::default();
    }
    let sources = Sources {
        generic,
        host: &host,
        page: &page,
    };
    let mut removed = Vec::new();
    let mut candidates = Candidates::default();
    let mut gone = BTreeSet::new();
    let mut tests = 0_u64;
    for (index, (root, ancestor)) in roots.iter().enumerate() {
        if ancestor.is_some_and(|ancestor| gone_with_an_ancestor(&ancestor, &gone)) {
            continue;
        }
        // One pass over each root finds, in document order, each element a
        // plain rule matches, credited to the first rule that matches it:
        // the earliest, in selector order, of those the generic set, the
        // host's rules and the page's own file under the element's keys,
        // each selector applied as the first list and rank holding it. The
        // matches of one root share their caches, since nothing leaves the
        // tree before the pass has collected them all.
        let mut caches = SelectorCaches::default();
        let found: Vec<(NodeRef<'_>, &str, List)> = root
            .descendants_it()
            .filter(NodeRef::is_element)
            .filter_map(|node| {
                let (rule, list) =
                    candidates
                        .of(&node, sources)
                        .iter()
                        .find_map(|(at, list, _)| {
                            let matcher = match *at {
                                At::Generic(at) => generic.matcher(at, selector),
                                At::Host(at) => host.rules[at].matcher.as_ref(),
                                At::Page(at) => page.rules[at].kind.as_ref(),
                            }?;
                            tests += 1;
                            matcher
                                .match_element_with_caches(&node, &mut caches)
                                .then_some((sources.selector(*at), *list))
                        })?;
                Some((node, rule, list))
            })
            .collect();
        tests += host.procedural.len() as u64;
        let procedurally = host.procedural.iter().flat_map(|rule| {
            matches(root, &rule.kind)
                .into_iter()
                .map(|node| (node, rule.name.as_str(), rule.list))
        });
        for (node, rule, list) in found.into_iter().chain(procedurally) {
            if gone_with_an_ancestor(&node, &gone) {
                continue;
            }
            removed.push(Removed {
                rule: rule.to_owned(),
                list: Some(list),
                in_shadow_root: index > 0,
                text: visible_length(&node.text()),
                links: links(&node),
            });
            gone.insert(node.id);
            node.remove_from_parent();
        }
    }
    work.count_many(Row::PageRuleTests, tests);
    Removal {
        removed_text: removed.iter().map(|element| element.text).sum(),
        removed,
    }
}

/// The generic rules a class or ID keys that apply to one page: those its
/// classes and IDs key, of each list `host` applies them for, less the
/// host's exceptions, in selector order, then list order, filed in an
/// index.
struct ClassAndIdRules {
    rules: Vec<Rule<Option<Matcher>>>,
    index: Index,
}

/// The page's class and ID rules, of `lists` as `host` applies them,
/// for the classes and IDs `roots` hold, compiled through `selector`.
fn class_and_id_rules(
    lists: &FilterLists,
    host: &Host,
    roots: &[(NodeRef<'_>, Option<NodeRef<'_>>)],
    selector: &impl Fn(&str) -> Option<Matcher>,
) -> ClassAndIdRules {
    let mut rules: Vec<(String, List)> = Vec::new();
    if !host.exceptions.is_empty() {
        let (classes, ids) = classes_and_ids(roots.iter().map(|(root, _)| root));
        for (list, exceptions) in &host.exceptions {
            let engine = lists.engine(*list);
            rules.extend(
                engine
                    .hidden_class_id_selectors(classes.iter(), ids.iter(), exceptions)
                    .into_iter()
                    .map(|selector| (selector, *list)),
            );
        }
    }
    rules.sort_unstable();
    rules.dedup_by(|later, earlier| later.0 == earlier.0);
    let rules: Vec<Rule<Option<Matcher>>> = rules
        .into_iter()
        .map(|(name, list)| Rule {
            kind: selector(&name),
            name,
            list,
        })
        .collect();
    let index = Index::of(rules.iter().map(|rule| rule.name.as_str()));
    ClassAndIdRules { rules, index }
}

/// Where a candidate rule is held.
#[derive(Clone, Copy)]
enum At {
    Generic(usize),
    Host(usize),
    Page(usize),
}

/// The rules one page is read with: the generic set, its host's and its
/// own class and ID rules.
#[derive(Clone, Copy)]
struct Sources<'r> {
    generic: &'r Generic,
    host: &'r Host,
    page: &'r ClassAndIdRules,
}

impl<'r> Sources<'r> {
    /// The selector of the rule at `at`.
    fn selector(self, at: At) -> &'r str {
        match at {
            At::Generic(at) => self.generic.rule(at).0,
            At::Host(at) => &self.host.rules[at].selector,
            At::Page(at) => &self.page.rules[at].name,
        }
    }
}

/// The scratch space of an element's candidates, reused element to
/// element.
#[derive(Default)]
struct Candidates {
    generic: Vec<usize>,
    host: Vec<usize>,
    page: Vec<usize>,
    /// Each candidate rule, with the list and rank its selector is applied
    /// as.
    all: Vec<(At, List, u8)>,
}

impl Candidates {
    /// The rules `node` could match, each selector once, as the first list
    /// and rank holding it, in selector order.
    fn of(&mut self, node: &NodeRef<'_>, sources: Sources<'_>) -> &[(At, List, u8)] {
        let Sources {
            generic,
            host,
            page,
        } = sources;
        self.all.clear();
        if !host.generic.is_empty() {
            generic.index().candidates(node, &mut self.generic);
            self.all.extend(self.generic.iter().filter_map(|at| {
                let list = host.generic_list(*at, generic.rule(*at).1)?;
                Some((At::Generic(*at), list, 0))
            }));
        }
        host.index.candidates(node, &mut self.host);
        self.all.extend(self.host.iter().map(|at| {
            let rule = &host.rules[*at];
            (At::Host(*at), rule.list, rule.rank)
        }));
        page.index.candidates(node, &mut self.page);
        self.all.extend(
            self.page
                .iter()
                .map(|at| (At::Page(*at), page.rules[*at].list, 1)),
        );
        self.all
            .sort_unstable_by(|(a, a_list, a_rank), (b, b_list, b_rank)| {
                (sources.selector(*a), a_list, a_rank).cmp(&(sources.selector(*b), b_list, b_rank))
            });
        self.all.dedup_by(|(later, ..), (earlier, ..)| {
            sources.selector(*later) == sources.selector(*earlier)
        });
        &self.all
    }
}

/// Removes from `document` too, in place, every element `selectors`
/// match, in the document and its shadow roots: the selectors of the rules
/// the Consent prompt runner hid prompts with, each element added to
/// `removal` with no list. A selector that does not parse is left out.
pub fn remove_selected(
    document: &ParsedHtml,
    removal: &mut Removal,
    selectors: &[String],
    work: &CountedWork,
) {
    let matchers: Vec<(&str, Matcher)> = selectors
        .iter()
        .filter_map(|selector| {
            Some((
                selector.as_str(),
                work.selector(Row::PageSelectorCompilations, selector)?,
            ))
        })
        .collect();
    if matchers.is_empty() {
        return;
    }
    let roots = roots(document, work);
    let mut gone = BTreeSet::new();
    for (index, (root, host)) in roots.iter().enumerate() {
        if host.is_some_and(|host| gone_with_an_ancestor(&host, &gone)) {
            continue;
        }
        for (selector, matcher) in &matchers {
            for node in Selection::from(*root).select_matcher(matcher).nodes() {
                if gone_with_an_ancestor(node, &gone) {
                    continue;
                }
                let text = visible_length(&node.text());
                removal.removed.push(Removed {
                    rule: (*selector).to_owned(),
                    list: None,
                    in_shadow_root: index > 0,
                    text,
                    links: links(node),
                });
                removal.removed_text += text;
                gone.insert(node.id);
                node.remove_from_parent();
            }
        }
    }
}

/// The document's root, then each shadow root with the `<template>` that
/// holds it, breadth first in document order.
fn roots<'d>(
    document: &'d ParsedHtml,
    work: &CountedWork,
) -> Vec<(NodeRef<'d>, Option<NodeRef<'d>>)> {
    let shadow = work.literal_selector(Row::PageSelectorCompilations, "template[shadowrootmode]");
    let mut roots = vec![(document.root(), None)];
    let mut next = 0;
    while next < roots.len() {
        let (root, _) = roots[next];
        next += 1;
        for template in Selection::from(root).select_matcher(&shadow).nodes() {
            let contents =
                template.query(|node| node.as_element().and_then(|e| e.template_contents));
            if let Some(contents) = contents.flatten().and_then(|id| document.tree.get(&id)) {
                roots.push((contents, Some(*template)));
            }
        }
    }
    roots
}

/// Every class and ID the roots' elements hold.
fn classes_and_ids<'a, 'b: 'a>(
    roots: impl Iterator<Item = &'a NodeRef<'b>>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut classes, mut ids) = (BTreeSet::new(), BTreeSet::new());
    for root in roots {
        for node in root.descendants_it() {
            if let Some(class) = node.class() {
                classes.extend(class.split_whitespace().map(str::to_owned));
            }
            if let Some(id) = node.id_attr()
                && !id.is_empty()
            {
                ids.insert(id.to_string());
            }
        }
    }
    (classes, ids)
}

/// The rule removal makes of a procedural `filter` of `list`: none for an
/// action other than hiding or removing, or an operator it does not
/// evaluate. Its selectors are compiled through `selector`.
fn procedural(
    filter: &ProceduralOrActionFilter,
    list: List,
    selector: &impl Fn(&str) -> Option<Matcher>,
) -> Option<Rule<Procedural>> {
    match filter.action {
        None | Some(CosmeticFilterAction::Remove) => {}
        Some(_) => return None,
    }
    let steps = filter
        .selector
        .iter()
        .map(|operator| match operator {
            CosmeticFilterOperator::CssSelector(text) => selector(text).map(Step::Css),
            CosmeticFilterOperator::HasText(text) if !is_pattern(text) => {
                Some(Step::HasText(text.clone()))
            }
            CosmeticFilterOperator::MinTextLength(length) => {
                length.parse().ok().map(Step::MinTextLength)
            }
            CosmeticFilterOperator::Upward(by) => match by.parse() {
                Ok(levels) => Some(Step::UpwardBy(levels)),
                Err(_) => selector(by).map(Step::UpwardTo),
            },
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if !matches!(steps.first(), Some(Step::Css(_))) {
        return None;
    }
    Some(Rule {
        name: serde_json::to_string(filter).unwrap_or_default(),
        list,
        kind: Procedural(steps),
    })
}

/// Whether a `:has-text` argument is a regular expression, which removal
/// does not evaluate.
fn is_pattern(text: &str) -> bool {
    text.len() > 1 && text.starts_with('/') && text.ends_with('/')
}

/// The elements the procedural rule `steps` matches under `root`.
fn matches<'a>(root: &NodeRef<'a>, Procedural(steps): &Procedural) -> Vec<NodeRef<'a>> {
    let mut found: Vec<NodeRef<'a>> = vec![*root];
    for (index, step) in steps.iter().enumerate() {
        found = match step {
            Step::Css(matcher) if index == 0 => Selection::from(*root)
                .select_matcher(matcher)
                .nodes()
                .to_vec(),
            Step::Css(matcher) => found
                .iter()
                .flat_map(|node| {
                    Selection::from(*node)
                        .select_matcher(matcher)
                        .nodes()
                        .to_vec()
                })
                .collect(),
            Step::HasText(text) => found
                .into_iter()
                .filter(|node| node.text().contains(text.as_str()))
                .collect(),
            Step::MinTextLength(length) => found
                .into_iter()
                .filter(|node| node.text().chars().count() >= *length)
                .collect(),
            Step::UpwardBy(levels) => found
                .iter()
                .filter_map(|node| {
                    node.ancestors_it(Some(*levels))
                        .nth(levels.saturating_sub(1))
                })
                .filter(NodeRef::is_element)
                .collect(),
            Step::UpwardTo(matcher) => found
                .iter()
                .filter_map(|node| {
                    node.ancestors_it(None)
                        .find(|ancestor| ancestor.is_match(matcher))
                })
                .collect(),
        };
    }
    let mut seen = BTreeSet::new();
    found.retain(|node| seen.insert(node.id));
    found
}

/// Whether `node`, or an ancestor of it, was removed already.
fn gone_with_an_ancestor(node: &NodeRef<'_>, gone: &BTreeSet<NodeId>) -> bool {
    gone.contains(&node.id)
        || node
            .ancestors_it(None)
            .any(|ancestor| gone.contains(&ancestor.id))
}

/// The `href` of each link `node` holds, itself included.
fn links(node: &NodeRef<'_>) -> Vec<String> {
    std::iter::once(*node)
        .chain(node.descendants_it())
        .filter(|node| {
            node.node_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("a"))
        })
        .filter_map(|link| link.attr("href").map(|href| href.to_string()))
        .collect()
}

/// The length of `text`, white space aside, in characters.
fn visible_length(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

/// How much of a page's main content's text a removal took: the main
/// content's text before removal and after, white space aside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Share {
    /// The main content's text of the page as served.
    pub before: usize,
    /// The main content's text once removal ran.
    pub after: usize,
}

impl Share {
    /// The share of a main content of `before` characters that one of
    /// `after` keeps.
    pub fn of(before: usize, after: usize) -> Self {
        Self { before, after }
    }

    /// The share taken, from 0 to 1; none taken of an empty main content.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a page's text is far below the 2^52 characters an f64 holds exactly"
    )]
    pub fn taken(&self) -> f64 {
        if self.before == 0 {
            return 0.0;
        }
        self.before.saturating_sub(self.after) as f64 / self.before as f64
    }

    /// Whether the share taken is above [`FLAGGED_SHARE`].
    pub fn flagged(&self) -> bool {
        self.taken() > FLAGGED_SHARE
    }
}
