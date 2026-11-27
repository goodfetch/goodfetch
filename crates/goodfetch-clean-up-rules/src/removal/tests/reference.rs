//! The removal as it was before the index (spec #2164, #2185): every
//! element tested against every plain rule of its page, in rule order, the
//! first match winning, then each procedural rule evaluated lazily on the
//! tree the removals before it left. Kept verbatim, beside the helpers it
//! shares with the removal, as what the index must agree with.

use std::collections::{BTreeMap, BTreeSet};

use dom_query::{Matcher, NodeRef};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

use super::super::{
    Procedural, Removal, Removed, Rule, Rules, classes_and_ids, gone_with_an_ancestor, links,
    matches, procedural, roots, visible_length,
};
use crate::cosmetic::{self, HIDDEN};
use crate::list::List;

/// Collects the plain hiding rules and procedural rules of `active` lists
/// (both live and removal-only cookie engines) for `url` and `(classes, ids)`.
fn collect_rules(
    rules: Rules<'_>,
    url: &str,
    active: &[List],
    (classes, ids): (&BTreeSet<String>, &BTreeSet<String>),
    selector: &impl Fn(&str) -> Option<Matcher>,
) -> (Vec<Rule<Matcher>>, Vec<Rule<Procedural>>) {
    let mut by_selector: BTreeMap<String, (String, List)> = BTreeMap::new();
    let mut procedurals = Vec::new();
    for &list in active {
        let live = rules.lists.engine(list);
        let live_res = live.url_cosmetic_resources(url);
        let rem = rules.lists.removal_only(list);
        let rem_res = rem.map(|extra| extra.url_cosmetic_resources(url));
        let mut merged_exceptions = live_res.exceptions.clone();
        if let Some(extra) = &rem_res {
            merged_exceptions.extend(extra.exceptions.iter().cloned());
        }
        let mut add = |selector: String, style: String| {
            by_selector.entry(selector).or_insert((style, list));
        };
        for (engine, resources, generic_hide) in [(live, &live_res, live_res.generichide)]
            .into_iter()
            .chain(rem.zip(rem_res.as_ref()).map(|(extra, extra_res)| {
                (
                    extra,
                    extra_res,
                    rules.lists.removal_generic_hide(list, url),
                )
            }))
        {
            #[expect(
                clippy::iter_over_hash_type,
                reason = "each selector goes into an ordered map, so the order read is not the order kept"
            )]
            for selector in &resources.hide_selectors {
                if !merged_exceptions.contains(selector) {
                    add(selector.clone(), HIDDEN.to_owned());
                }
            }
            if !generic_hide {
                for selector in
                    engine.hidden_class_id_selectors(classes.iter(), ids.iter(), &merged_exceptions)
                {
                    add(selector, HIDDEN.to_owned());
                }
            }
            for filter in cosmetic::procedural(resources) {
                match filter.as_css() {
                    Some((selector, style)) => add(selector, style),
                    None => procedurals.extend(procedural(&filter, list, selector)),
                }
            }
        }
    }
    let css = by_selector
        .into_iter()
        .filter(|(_, (style, _))| style == HIDDEN)
        .filter_map(|(name, (_, list))| {
            Some(Rule {
                kind: selector(&name)?,
                name,
                list,
            })
        })
        .collect();
    (css, procedurals)
}

/// [`super::super::remove`], before the index.
pub(super) fn remove(
    document: &ParsedHtml,
    url: &str,
    rules: Rules<'_>,
    work: &CountedWork,
) -> Removal {
    let active: Vec<List> = [
        (List::Ads, rules.choices.ads),
        (List::Trackers, rules.choices.trackers),
        (List::Annoyances, rules.choices.annoyances),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(list, _)| list)
    .collect();
    if active.is_empty() {
        return Removal::default();
    }
    work.count(Row::PageRuleListBuilds);
    let selector =
        |text: &str| work.rules_selector(Row::PageSelectorCompilations, rules.version, text);
    let roots = roots(document, work);
    let (classes, ids) = classes_and_ids(roots.iter().map(|(root, _)| root));
    let (css, procedural) = collect_rules(rules, url, &active, (&classes, &ids), &selector);
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
