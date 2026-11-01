//! A document as a browser serializes it, its shadow roots as
//! `<template shadowrootmode>` (ADR 0121), read as a person sees it: each
//! shadow root's text in its host's place, its slots filled with the
//! host's own children; and the Clean-up rules' cosmetic matcher, which the
//! call's `Context` carries (ADR 0047), applied before the main-content
//! step and the Markdown converter.

use std::cell::RefCell;

use dom_query::Document as Dom;
use goodfetch_protocol::page::FormatsRequest;
use serde_json::json;

use super::URL;
use crate::{CleanUp, Context, Document, Serialization, formats};

/// An article long enough for the main-content step, with `inside` after
/// its opening paragraphs.
fn article(inside: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><title>Kites</title></head><body><main><article>\
         <h1>How a kite stays in the air</h1>\
         <p>A kite flies because the wind pushes on its sail at an angle, and the line holds it against that push, so the kite is lifted as well as pushed.</p>\
         <p>The bridle is the set of lines that joins the flying line to the frame, and moving its towing point changes how steeply the kite climbs.</p>\
         {inside}\
         <p>Most flyers adjust the bridle a little at a time, letting the kite settle for a minute after each change before judging the result.</p>\
         </article></main></body></html>"
    )
}

/// The Markdown of `html`, read at [`URL`] with `context`.
fn markdown(html: &str, context: &Context<'_>) -> String {
    let request: FormatsRequest = serde_json::from_value(json!({ "markdown": true })).unwrap();
    formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Dom,
        },
        &request,
        context,
    )
    .markdown
    .unwrap()
}

/// Covers: story:6-9
#[test]
fn the_text_of_open_and_closed_shadow_roots_is_read_in_their_hosts_place() {
    let html = article(
        "<div><template shadowrootmode=\"open\"><p>Text in an open root, which a person reads.</p></template></div>\
         <div><template shadowrootmode=\"closed\"><p>Text in a closed root, which a person reads too.</p></template></div>",
    );

    let read = markdown(&html, &Context::default());

    assert!(
        read.contains(
            "The bridle is the set of lines that joins the flying line to the frame, and moving its towing point changes how steeply the kite climbs.\n\nText in an open root, which a person reads.\n\nText in a closed root, which a person reads too.\n\nMost flyers"
        ),
        "{read}"
    );
}

/// Covers: story:6-9
#[test]
fn a_hosts_children_fill_its_shadow_roots_slots_and_one_no_slot_takes_is_unseen() {
    let html = article(
        "<div>\
         <template shadowrootmode=\"closed\"><p>Before the slots.</p><slot name=\"title\"></slot><slot><p>A default slot's own fallback text.</p></slot><slot name=\"empty\"><p>A named slot's fallback text.</p></slot></template>\
         <p>A child the default slot takes.</p>\
         <p slot=\"title\">A child the title slot takes.</p>\
         <p slot=\"nowhere\">A child no slot takes.</p>\
         </div>",
    );

    let read = markdown(&html, &Context::default());

    assert!(
        read.contains(
            "Before the slots.\n\nA child the title slot takes.\n\nA child the default slot takes.\n\nA named slot's fallback text.\n\n"
        ) && !read.contains("no slot takes")
            && !read.contains("default slot's own fallback"),
        "{read}"
    );
}

/// Covers: story:6-9
#[test]
fn a_shadow_root_inside_another_is_read_in_its_hosts_place() {
    let html = article(
        "<div><template shadowrootmode=\"open\"><p>The outer root's text.</p>\
         <div><template shadowrootmode=\"closed\"><p>The inner root's text.</p><slot></slot></template><p>The inner host's child.</p></div>\
         </template></div>",
    );

    let read = markdown(&html, &Context::default());

    assert!(
        read.contains(
            "The outer root's text.\n\nThe inner root's text.\n\nThe inner host's child.\n\n"
        ),
        "{read}"
    );
}

/// A cosmetic matcher of its own: removes every `.ad-slot` element, of the
/// document and of its shadow roots, and keeps the HTML it was given.
#[derive(Default)]
struct AdSlots {
    given: RefCell<Vec<String>>,
}

impl CleanUp for AdSlots {
    fn remove(&self, html: &str, url: &str) -> String {
        assert_eq!(url, URL);
        self.given.borrow_mut().push(html.to_owned());
        let document = Dom::from(html);
        document.select(".ad-slot").remove();
        for template in document.select("template[shadowrootmode]").nodes() {
            let contents = template
                .query(|node| {
                    node.as_element()
                        .and_then(|element| element.template_contents)
                })
                .flatten()
                .and_then(|id| document.tree.get(&id));
            if let Some(contents) = contents {
                dom_query::Selection::from(contents)
                    .select(".ad-slot")
                    .remove();
            }
        }
        document.html().to_string()
    }
}

/// Covers: story:6-10
#[test]
fn the_cosmetic_matcher_takes_its_elements_out_of_the_markdown_shadow_roots_included() {
    let html = article(
        "<div class=\"ad-slot\">A light ad.</div>\
         <div><template shadowrootmode=\"closed\"><p>Text in a closed root, which a person reads.</p><div class=\"ad-slot\">A closed-root ad.</div></template></div>",
    );
    let matcher = AdSlots::default();

    let read = markdown(
        &html,
        &Context {
            clean_up: Some(&matcher),
            ..Context::default()
        },
    );

    assert!(
        read.contains("Text in a closed root, which a person reads.")
            && !read.contains("ad.")
            && matcher.given.borrow().as_slice() == [html.clone()],
        "{read}"
    );
}

/// Covers: story:6-10
#[test]
fn without_a_cosmetic_matcher_nothing_is_taken_out() {
    let html = article("<div class=\"ad-slot\">A light ad, which no rule matches here.</div>");

    let read = markdown(&html, &Context::default());

    assert!(
        read.contains("A light ad, which no rule matches here."),
        "{read}"
    );
}
