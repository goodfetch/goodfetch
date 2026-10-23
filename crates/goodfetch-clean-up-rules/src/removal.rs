//! Extraction-side removal (ADR 0047): before dom-smoothie, htmd or any AI
//! input, the elements the cosmetic rules of the lists that are on match
//! go from a page's HTML, in every tier, inside its shadow roots too (a
//! page serialized with its shadow roots holds each, open or closed, as a
//! `<template shadowrootmode>`), so hidden text never reaches extracted
//! output. Procedural rules apply here where live hiding cannot: those
//! that select by CSS, by their text (`:has-text`, `:min-text-length`) and
//! by an ancestor (`:upward`); the rest are left out. The caller learns
//! what went, each element with its rule, its text and the links inside it,
//! which area 6's trap-link judgement leaves out (ADR 0035). A removal
//! over a fifth of the main content's text is flagged ([`Share`]).

use std::collections::BTreeSet;

use adblock::cosmetic_filter_cache::ProceduralOrActionFilter;
use adblock::filters::cosmetic::{CosmeticFilterAction, CosmeticFilterOperator};
use dom_query::{Document, Matcher, NodeId, NodeRef, Selection};

use crate::cosmetic::{HIDDEN, PageRules};
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
    /// The list the rule comes from.
    pub list: List,
    /// Whether it sat in a shadow root.
    pub in_shadow_root: bool,
    /// Its text, white space aside, in characters.
    pub text: usize,
    /// The `href` of each link it held, itself included.
    pub links: Vec<String>,
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
/// the cosmetic rules of the lists `choices` turn on, among `lists`, match.
pub fn remove(html: &str, url: &str, lists: &FilterLists, choices: &Choices) -> Removal {
    let rules = PageRules::of(lists, choices, url);
    if rules.is_empty() {
        return untouched(html);
    }
    let document = Document::from(html);
    let roots = roots(&document);
    let (classes, ids) = classes_and_ids(roots.iter().map(|(root, _)| root));
    let css: Vec<Rule<Matcher>> = rules
        .css_rules(
            &classes.iter().map(String::as_str).collect::<Vec<_>>(),
            &ids.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .into_iter()
        .filter(|rule| rule.style == HIDDEN)
        .filter_map(|rule| {
            Some(Rule {
                kind: Matcher::new(&rule.selector).ok()?,
                name: rule.selector,
                list: rule.list,
            })
        })
        .collect();
    // One pass over each root finds what any plain rule matches; each
    // element found is then credited to the first rule that matches it.
    let every = Matcher::new(
        &css.iter()
            .map(|rule| rule.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
    .ok();
    let procedural: Vec<Rule<Procedural>> = rules
        .procedural()
        .into_iter()
        .filter_map(|(filter, list)| procedural(&filter, list))
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
        let found: Vec<(NodeRef<'_>, &str, List)> = match &every {
            Some(every) => Selection::from(*root)
                .select_matcher(every)
                .nodes()
                .iter()
                .filter_map(|node| {
                    let rule = css.iter().find(|rule| node.is_match(&rule.kind))?;
                    Some((*node, rule.name.as_str(), rule.list))
                })
                .collect(),
            None => css
                .iter()
                .flat_map(|rule| {
                    Selection::from(*root)
                        .select_matcher(&rule.kind)
                        .nodes()
                        .iter()
                        .map(|node| (*node, rule.name.as_str(), rule.list))
                        .collect::<Vec<_>>()
                })
                .collect(),
        };
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
                list,
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
fn roots(document: &Document) -> Vec<(NodeRef<'_>, Option<NodeRef<'_>>)> {
    let mut roots = vec![(document.root(), None)];
    let mut next = 0;
    while next < roots.len() {
        let (root, _) = roots[next];
        next += 1;
        let shadow = Matcher::new("template[shadowrootmode]")
            .expect("invariant: the shadow root selector is CSS");
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
/// evaluate.
fn procedural(filter: &ProceduralOrActionFilter, list: List) -> Option<Rule<Procedural>> {
    match filter.action {
        None | Some(CosmeticFilterAction::Remove) => {}
        Some(_) => return None,
    }
    let steps = filter
        .selector
        .iter()
        .map(|operator| match operator {
            CosmeticFilterOperator::CssSelector(selector) => {
                Matcher::new(selector).ok().map(Step::Css)
            }
            CosmeticFilterOperator::HasText(text) if !is_pattern(text) => {
                Some(Step::HasText(text.clone()))
            }
            CosmeticFilterOperator::MinTextLength(length) => {
                length.parse().ok().map(Step::MinTextLength)
            }
            CosmeticFilterOperator::Upward(by) => match by.parse() {
                Ok(levels) => Some(Step::UpwardBy(levels)),
                Err(_) => Matcher::new(by).ok().map(Step::UpwardTo),
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
