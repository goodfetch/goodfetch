//! The formats a request names, read from one document: the interface every
//! tier calls with the document it holds, and the composition of the
//! main-content step and the converters behind it.

use std::fmt::{self, Debug, Formatter};

use crate::main_content::{main_content, main_text_length};
use crate::markdown::markdown;
use crate::shadow::composed;
use goodfetch_protocol::page::{Asked, FormatsRequest, MainContent, MarkdownFormat, PageFormats};

/// A page's document as a tier serialized it: its HTML as text, decoded
/// already, its URL after redirects and its base URL (its first
/// `<base href>`, else its URL).
#[derive(Clone, Copy, Debug)]
pub struct Document<'a> {
    /// Its HTML.
    pub html: &'a str,
    /// Its URL.
    pub url: &'a str,
    /// Its base URL.
    pub base_url: &'a str,
}

/// The Clean-up rules' cosmetic matcher (ADR 0047), area 5's, which a
/// call's [`Context`] carries: it takes out of a document's HTML the
/// elements the cosmetic rules match, inside its shadow roots too (each a
/// `<template shadowrootmode>`).
pub trait CleanUp {
    /// `html`, the document of the page at `url`, without the elements the
    /// cosmetic rules match.
    fn remove(&self, html: &str, url: &str) -> String;
}

/// What a call reads with beside its document: the Clean-up rules'
/// cosmetic matcher, when the page's rules are on. The `content.extractor`
/// setting's choice joins it (#788).
#[derive(Clone, Copy, Default)]
pub struct Context<'a> {
    /// The cosmetic matcher applied before the main-content step and the
    /// Markdown converter, never to what reads the document as it stands;
    /// none applies no rule.
    pub clean_up: Option<&'a dyn CleanUp>,
}

impl Debug for Context<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Context")
            .field("clean_up", &self.clean_up.is_some())
            .finish()
    }
}

/// The formats `request` names, read from `document`, and no other: a
/// document with no main content answers each format empty and
/// `main_content: none`, never the whole page in its place.
pub fn formats(
    document: &Document<'_>,
    request: &FormatsRequest,
    context: &Context<'_>,
) -> PageFormats {
    // Formats of the document as it stands (`html.raw`, links) read
    // `document.html`; the main content and its Markdown read it once the
    // cosmetic rules took their elements out, then as a person sees it.
    let cleaned = context
        .clean_up
        .map(|clean_up| clean_up.remove(document.html, document.url));
    let seen = composed(cleaned.as_deref().unwrap_or(document.html));
    let main = main_content(&seen, document.url);
    PageFormats {
        url: document.url.to_owned(),
        base_url: document.base_url.to_owned(),
        main_content: if main.is_some() {
            MainContent::Found
        } else {
            MainContent::None
        },
        markdown: request
            .markdown
            .map(|_| main.as_deref().map(markdown).unwrap_or_default()),
    }
}

/// The length of the text of `document`'s main content, white space
/// aside, in characters, its shadow roots' text included; 0 when it has
/// none: how a caller measures what extraction-side removal took of it
/// (ADR 0047).
pub fn main_content_length(document: &Document<'_>) -> usize {
    main_text_length(&composed(document.html), document.url)
}

/// The fuzz target of the HTML entry point (ADR 0067): reads `data` as a
/// document's HTML, lossily decoded, with every format asked. It panics only
/// on a bug.
pub fn fuzz_formats(data: &[u8]) {
    let html = String::from_utf8_lossy(data);
    let document = Document {
        html: &html,
        url: "https://fuzz.test/page",
        base_url: "https://fuzz.test/page",
    };
    let request = FormatsRequest {
        markdown: Some(MarkdownFormat::Asked(Asked)),
    };
    let _ = formats(&document, &request, &Context::default());
}

#[cfg(test)]
mod tests;
