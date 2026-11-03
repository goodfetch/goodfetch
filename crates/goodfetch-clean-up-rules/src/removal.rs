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
//! main content's text is flagged ([`Share`]). A Run's pages share the
//! selectors it parsed ([`Selectors`]), so each is parsed once.
#![expect(
    clippy::disallowed_types,
    reason = "the parsed selectors' lock guards one map for one page's lookups, on a blocking thread, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeSet, HashMap};
use std::sync::{LazyLock, Mutex, PoisonError};

use adblock::cosmetic_filter_cache::ProceduralOrActionFilter;
use adblock::filters::cosmetic::{CosmeticFilterAction, CosmeticFilterOperator};
use dom_query::{Matcher, NodeId, NodeRef, Selection};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

use crate::cosmetic::{CssRule, HIDDEN, PageRules};
use crate::matching::{Choices, List};
use crate::rules::FilterLists;

/// The share of the main content's text above which a removal is flagged
/// in Provenance (ADR 0047's share, which spec #577 sets): a fifth.
pub const FLAGGED_SHARE: f64 = 0.20;

/// What removal took from a document.
#[derive(Clone, Debug)]
pub struct Removal {
    /// The document without them, serialized.
    pub html: String,
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

/// The selectors removal parsed for the pages that share it, a Run's: each
/// is parsed on the first page whose rules hold it and kept, never parsed
/// again for a later page (#1765). Parsing the hundreds of selectors each
/// page's rules hold cost more than the rest of its removal. A selector
/// that does not parse is kept as such. It holds at most every selector of
/// the lists, and goes with the last page that shares it.
#[derive(Default)]
pub struct Selectors {
    parsed: Mutex<HashMap<String, Option<Matcher>>>,
    /// Where a removal counts its parse of a document (ADR 0140).
    work: CountedWork,
}

impl Selectors {
    /// None parsed yet, a removal through it counting on `work`.
    #[must_use]
    pub fn counting(work: CountedWork) -> Self {
        Self {
            parsed: Mutex::default(),
            work,
        }
    }

    /// How many selectors it parsed so far.
    pub fn parsed(&self) -> usize {
        self.parsed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// The matcher of each of `selectors`, in their order; none for one
    /// that does not parse.
    fn matchers<'s>(&self, selectors: impl IntoIterator<Item = &'s str>) -> Vec<Option<Matcher>> {
        let mut parsed = self.parsed.lock().unwrap_or_else(PoisonError::into_inner);
        selectors
            .into_iter()
            .map(|selector| {
                if let Some(matcher) = parsed.get(selector) {
                    return matcher.clone();
                }
                let matcher = self
                    .work
                    .compile_selector(Row::PageSelectorCompilations, selector);
                parsed.insert(selector.to_owned(), matcher.clone());
                matcher
            })
            .collect()
    }

    /// The matcher of `selector`, if it parses.
    fn matcher(&self, selector: &str) -> Option<Matcher> {
        self.matchers([selector]).pop().flatten()
    }
}

/// One rule as removal applies it.
struct Rule<K> {
    name: String,
    list: List,
    kind: K,
}

/// A procedural rule's operators, in order.
struct Procedural(Vec<Step>);

/// One procedural operator removal evaluates.
enum Step {
    Css(Matcher),
    HasText(String),
    MinTextLength(usize),
    UpwardBy(usize),
    UpwardTo(Matcher),
}

/// Removes from `html`, the document of the page at `url`, every element
/// the cosmetic rules of the lists `choices` turn on, among `lists`, match,
/// their selectors parsed through `selectors`.
pub fn remove(
    html: &str,
    url: &str,
    (lists, choices): (&FilterLists, &Choices),
    selectors: &Selectors,
) -> Removal {
    let rules = PageRules::of(lists, choices, url);
    if rules.is_empty() {
        return untouched(html);
    }
    let document = selectors.work.parse_html(Row::PageParses, html);
    let roots = roots(&document);
    let (classes, ids) = classes_and_ids(roots.iter().map(|(root, _)| root));
    let hiding: Vec<CssRule> = rules
        .css_rules(
            &classes.iter().map(String::as_str).collect::<Vec<_>>(),
            &ids.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .into_iter()
        .filter(|rule| rule.style == HIDDEN)
        .collect();
    let matchers = selectors.matchers(hiding.iter().map(|rule| rule.selector.as_str()));
    let css: Vec<Rule<Matcher>> = hiding
        .into_iter()
        .zip(matchers)
        .filter_map(|(rule, matcher)| {
            Some(Rule {
                kind: matcher?,
                name: rule.selector,
                list: rule.list,
            })
        })
        .collect();
    let procedural: Vec<Rule<Procedural>> = rules
        .procedural()
        .into_iter()
        .filter_map(|(filter, list)| procedural(&filter, list, selectors))
        .collect();
    if css.is_empty() && procedural.is_empty() {
        return untouched(html);
    }
    let mut removed = Vec::new();
    let mut gone = BTreeSet::new();
    for (index, (root, host)) in roots.iter().enumerate() {
        if host.is_some_and(|host| gone_with_an_ancestor(&host, &gone)) {
            continue;
        }
        // One pass over each root finds, in document order, each element a
        // plain rule matches, credited to the first rule that matches it.
        let found: Vec<(NodeRef<'_>, &str, List)> = root
            .descendants_it()
            .filter(NodeRef::is_element)
            .filter_map(|node| {
                let rule = css.iter().find(|rule| node.is_match(&rule.kind))?;
                Some((node, rule.name.as_str(), rule.list))
            })
            .collect();
        let procedurally = procedural.iter().flat_map(|rule| {
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
    if removed.is_empty() {
        return untouched(html);
    }
    Removal {
        html: document.html().to_string(),
        removed_text: removed.iter().map(|element| element.text).sum(),
        removed,
    }
}

/// `removal` with every element `selectors` match removed too, in the
/// document and its shadow roots: the selectors of the rules the Consent
/// prompt runner hid prompts with, each element recorded with no list. A
/// selector that does not parse is left out.
#[expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub fn remove_selected(removal: Removal, selectors: &[String]) -> Removal {
    let matchers: Vec<(&str, Matcher)> = selectors
        .iter()
        .filter_map(|selector| Some((selector.as_str(), Matcher::new(selector).ok()?)))
        .collect();
    if matchers.is_empty() {
        return removal;
    }
    let Removal {
        html,
        mut removed,
        mut removed_text,
    } = removal;
    let document = dom_query::Document::from(html.as_str());
    let roots = roots(&document);
    let mut gone = BTreeSet::new();
    let mut took = false;
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
                removed.push(Removed {
                    rule: (*selector).to_owned(),
                    list: None,
                    in_shadow_root: index > 0,
                    text,
                    links: links(node),
                });
                removed_text += text;
                gone.insert(node.id);
                node.remove_from_parent();
                took = true;
            }
        }
    }
    Removal {
        html: if took {
            document.html().to_string()
        } else {
            html
        },
        removed,
        removed_text,
    }
}

/// `html` as removal leaves it when nothing matches.
fn untouched(html: &str) -> Removal {
    Removal {
        html: html.to_owned(),
        removed: Vec::new(),
        removed_text: 0,
    }
}

/// The document's root, then each shadow root with the `<template>` that
/// holds it, breadth first in document order.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn roots(document: &ParsedHtml) -> Vec<(NodeRef<'_>, Option<NodeRef<'_>>)> {
    static SHADOW: LazyLock<Matcher> = LazyLock::new(|| {
        Matcher::new("template[shadowrootmode]")
            .expect("invariant: the shadow root selector is CSS")
    });
    let mut roots = vec![(document.root(), None)];
    let mut next = 0;
    while next < roots.len() {
        let (root, _) = roots[next];
        next += 1;
        for template in Selection::from(root).select_matcher(&SHADOW).nodes() {
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
/// evaluate. Its selectors are parsed through `selectors`.
fn procedural(
    filter: &ProceduralOrActionFilter,
    list: List,
    selectors: &Selectors,
) -> Option<Rule<Procedural>> {
    match filter.action {
        None | Some(CosmeticFilterAction::Remove) => {}
        Some(_) => return None,
    }
    let steps = filter
        .selector
        .iter()
        .map(|operator| match operator {
            CosmeticFilterOperator::CssSelector(selector) => {
                selectors.matcher(selector).map(Step::Css)
            }
            CosmeticFilterOperator::HasText(text) if !is_pattern(text) => {
                Some(Step::HasText(text.clone()))
            }
            CosmeticFilterOperator::MinTextLength(length) => {
                length.parse().ok().map(Step::MinTextLength)
            }
            CosmeticFilterOperator::Upward(by) => match by.parse() {
                Ok(levels) => Some(Step::UpwardBy(levels)),
                Err(_) => selectors.matcher(by).map(Step::UpwardTo),
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
