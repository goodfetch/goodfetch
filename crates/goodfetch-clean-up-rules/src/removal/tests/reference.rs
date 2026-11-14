//! The removal as it was before the index (spec #2164, #2185): every
//! element tested against every plain rule of its page, in rule order, the
//! first match winning, then each procedural rule evaluated lazily on the
//! tree the removals before it left. Kept verbatim, beside the helpers it
//! shares with the removal, as what the index must agree with.

use std::collections::BTreeSet;

use dom_query::{Matcher, NodeRef};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

use super::super::{
    Procedural, Removal, Removed, Rule, Rules, classes_and_ids, gone_with_an_ancestor, links,
    matches, procedural, roots, visible_length,
};
use crate::cosmetic::{self, HIDDEN, PageRules};
use crate::list::List;

/// [`super::super::remove`], before the index.
pub(super) fn remove(
    document: &ParsedHtml,
    url: &str,
    rules: Rules<'_>,
    work: &CountedWork,
) -> Removal {
    let page = PageRules::of(rules.lists, rules.choices, url);
    if page.is_empty() {
        return Removal::default();
    }
    work.count(Row::PageRuleListBuilds);
    let selector =
        |text: &str| work.rules_selector(Row::PageSelectorCompilations, rules.version, text);
    let roots = roots(document, work);
    let (classes, ids) = classes_and_ids(roots.iter().map(|(root, _)| root));
    let css: Vec<Rule<Matcher>> = page
        .css_rules(
            &classes.iter().map(String::as_str).collect::<Vec<_>>(),
            &ids.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .into_iter()
        .filter(|rule| rule.style == HIDDEN)
        .filter_map(|rule| {
            Some(Rule {
                kind: selector(&rule.selector)?,
                name: rule.selector,
                list: rule.list,
            })
        })
        .collect();
    let procedural: Vec<Rule<Procedural>> = [
        (List::Ads, rules.choices.ads),
        (List::Trackers, rules.choices.trackers),
        (List::Annoyances, rules.choices.annoyances),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .flat_map(|(list, _)| {
        cosmetic::procedural(&rules.lists.engine(list).url_cosmetic_resources(url))
            .filter(|filter| filter.as_css().is_none())
            .map(move |filter| (filter, list))
            .collect::<Vec<_>>()
    })
    .filter_map(|(filter, list)| procedural(&filter, list, &selector))
    .collect();
    let mut removed = Vec::new();
    if css.is_empty() && procedural.is_empty() {
        return Removal::default();
    }
    let mut gone = BTreeSet::new();
    let mut tests = 0_u64;
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
                let rule = css.iter().find(|rule| {
                    tests += 1;
                    node.is_match(&rule.kind)
                })?;
                Some((node, rule.name.as_str(), rule.list))
            })
            .collect();
        tests += procedural.len() as u64;
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
    work.count_many(Row::PageRuleTests, tests);
    Removal {
        removed_text: removed.iter().map(|element| element.text).sum(),
        removed,
    }
}
