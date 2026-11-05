//! A document's composed tree (ADR 0121): a browser serializes each shadow
//! root, open or closed, as a `<template shadowrootmode>` inside its host,
//! which a parser keeps apart as the template's contents, so no text of it
//! would reach the main-content step. [`compose_in_place`] puts each shadow root's
//! content in its host's place, as a person sees the page: each `<slot>`
//! takes the host's children assigned to it (by their `slot` attribute, the
//! default slot those with none), else keeps its own fallback content, and
//! a host's child no slot takes is not shown.

use dom_query::{Matcher, NodeRef, Selection};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

/// The selector of a shadow root's template.
const SHADOW_ROOT: &str = "template[shadowrootmode]";

/// Puts every shadow root's content of `document` in its host's place, in
/// its tree (ADR 0138); whether it held any.
pub(crate) fn compose_in_place(document: &ParsedHtml, work: &CountedWork) -> bool {
    let templates = templates(
        document,
        &work.literal_selector(Row::PageSelectorCompilations, SHADOW_ROOT),
    );
    if templates.is_empty() {
        return false;
    }
    let slot = work.literal_selector(Row::PageSelectorCompilations, "slot");
    // The last found first: a shadow root inside another's content, and a
    // host among another host's children, is composed before the host that
    // holds it, so each host's children and content are composed already.
    for template in templates.iter().rev() {
        compose(document, template, &slot);
    }
    true
}

/// Every shadow root's template: those of the document in document order,
/// then those of each shadow root's content, breadth first.
fn templates<'a>(document: &'a ParsedHtml, matcher: &Matcher) -> Vec<NodeRef<'a>> {
    let mut found: Vec<NodeRef<'_>> = Selection::from(document.root())
        .select_matcher(matcher)
        .nodes()
        .to_vec();
    let mut next = 0;
    while next < found.len() {
        if let Some(contents) = contents(document, &found[next]) {
            found.extend(
                Selection::from(contents)
                    .select_matcher(matcher)
                    .nodes()
                    .iter()
                    .copied(),
            );
        }
        next += 1;
    }
    found
}

/// The content of the shadow root `template` holds.
fn contents<'a>(document: &'a ParsedHtml, template: &NodeRef<'a>) -> Option<NodeRef<'a>> {
    template
        .query(|node| {
            node.as_element()
                .and_then(|element| element.template_contents)
        })
        .flatten()
        .and_then(|id| document.tree.get(&id))
}

/// Puts the content of the shadow root `template` holds in its host's
/// place, its slots filled with the host's children.
fn compose(document: &ParsedHtml, template: &NodeRef<'_>, slot: &Matcher) {
    let (Some(host), Some(contents)) = (template.parent(), contents(document, template)) else {
        return;
    };
    let children: Vec<NodeRef<'_>> = host
        .children()
        .into_iter()
        .filter(|child| child.id != template.id)
        .collect();
    for found in Selection::from(contents).select_matcher(slot).nodes() {
        let name = found.attr("name").map(|name| name.to_string());
        let assigned: Vec<&NodeRef<'_>> = children
            .iter()
            .filter(|child| assigned_to(child, name.as_deref().unwrap_or_default()))
            .collect();
        if assigned.is_empty() {
            for fallback in found.children() {
                found.insert_before(&fallback);
            }
        } else {
            for child in assigned {
                found.insert_before(child);
            }
        }
        found.remove_from_parent();
    }
    host.remove_children();
    if let Some(first) = contents.first_child() {
        host.append_children(&first);
    }
}

/// Whether the host's child `child` is assigned to the slot named `name`,
/// the default slot being the one with no name: an element by its `slot`
/// attribute, text by having none.
fn assigned_to(child: &NodeRef<'_>, name: &str) -> bool {
    if child.is_element() {
        child
            .attr("slot")
            .map(|slot| slot.to_string())
            .unwrap_or_default()
            == name
    } else {
        child.is_text() && name.is_empty()
    }
}
