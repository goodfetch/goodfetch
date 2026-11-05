//! The formats a request names, read from one document: the interface every
//! tier calls with the document it holds, and the composition of the
//! main-content step and the converters behind it.

use std::borrow::Cow;
use std::fmt::{self, Debug, Formatter};

use crate::blocks::{body, fit, text};
use crate::contacts::contacts;
use crate::extractor::{BuiltIn, Extractor};
use crate::light::light;
use crate::main_content::{main_content, main_text_length};
use crate::markdown::markdown;
use crate::plan::{Demand, Plan, Reader};
use crate::references::{Head, References, references};
use crate::shadow::composed;
use crate::structured_data::{json_ld, microdata};
use crate::tables::tables;
use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::page::{
    Asked, FormatsRequest, HtmlFormat, HtmlKind, MainContent, MarkdownContent, MarkdownFormat,
    MarkdownOptions, Metadata, PageFormats,
};
use scraper::Html;
use url::Url;

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
    /// How it was serialized: as served, or from a browser's DOM.
    pub serialization: Serialization,
}

/// How a [`Document`]'s HTML was serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Serialization {
    /// As a site served it: a `<template shadowrootmode>` in it is the
    /// site's own markup.
    Served,
    /// From a browser's DOM as it stands, each shadow root, open or
    /// closed, serialized as a `<template shadowrootmode>` first in its
    /// host, beside the light tree `content()` serializes.
    Dom,
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
/// cosmetic matcher, when the page's rules are on, and the main-content
/// step `content.extractor` chose.
#[derive(Clone)]
pub struct Context<'a> {
    /// The cosmetic matcher applied before the main-content step and the
    /// Markdown converter, never to what reads the document as it stands;
    /// none applies no rule.
    pub clean_up: Option<&'a dyn CleanUp>,
    /// The main-content step.
    pub extractor: &'a dyn Extractor,
    /// Where the call's parses count (ADR 0140).
    pub work: CountedWork,
}

impl Default for Context<'_> {
    /// No Clean-up rule, and the default step, dom-smoothie.
    fn default() -> Self {
        Self {
            clean_up: None,
            extractor: BuiltIn::default().extractor(),
            work: CountedWork::default(),
        }
    }
}

impl Debug for Context<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Context")
            .field("clean_up", &self.clean_up.is_some())
            .finish_non_exhaustive()
    }
}

/// The formats `request` names, read from `document`, and no other: a
/// document with no main content answers each format read from it empty
/// and `main_content: none`, never the whole page in its place.
pub fn formats(
    document: &Document<'_>,
    request: &FormatsRequest,
    context: &Context<'_>,
) -> PageFormats {
    measured_formats(document, request, context).formats
}

/// What one call read: its formats, and the length of the text of the
/// main content they were read from, as [`main_content_length`] measures
/// it.
#[derive(Debug)]
pub struct Measured {
    /// The formats the request names.
    pub formats: PageFormats,
    /// The length of the main content's text, in characters, white space
    /// aside, of the document as the Clean-up rules left it; 0 when it has
    /// none, and none when the request names no format read from it.
    pub main_content_length: Option<usize>,
}

/// The formats `request` names, read from `document` as [`formats`] reads
/// them, and the length of the main content they were read from, so that a
/// caller measuring what the Clean-up rules took of it (ADR 0047) runs no
/// second main-content step on the document they left.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
pub fn measured_formats(
    document: &Document<'_>,
    request: &FormatsRequest,
    context: &Context<'_>,
) -> Measured {
    // The demand plan runs only the readers the request reads from (ADR
    // 0138): `html.raw`, links and the head read `document.html` as it
    // stands; every other format reads it once the cosmetic rules took
    // their elements out, and the Markdown and text then as a person sees
    // it. A request that reads nothing of the cleaned page runs no rule.
    let plan = Plan::of(&Demand {
        formats: request,
        learning: false,
    });
    let cleaned = context
        .clean_up
        .filter(|_| plan.runs(Reader::CleanUp))
        .map(|clean_up| clean_up.remove(document.html, document.url));
    let cleaned = cleaned.as_deref().unwrap_or(document.html);
    let seen = if plan.runs(Reader::CleanUp) {
        composed(cleaned)
    } else {
        Cow::Borrowed(cleaned)
    };
    // The main-content step runs only for a format read from its answer
    // (story 8): outside, none read; inside, none found.
    let read_main = plan
        .runs(Reader::MainContent)
        .then(|| main_content(&seen, document.url, context.extractor, &context.work));
    let main = read_main.as_ref().and_then(Option::as_ref);
    let main_html = main.map(|(html, _)| html.as_str());
    let of_main = |read: fn(&str) -> String| main_html.map(read).unwrap_or_default();
    // One streaming pass over the document as it stands, its shadow roots'
    // templates included, for its links and head; images and media are
    // extracted content, read after the Clean-up rules (ADR 0047), by a
    // second pass only when the rules changed the document.
    let raw_read = plan.runs(Reader::Untouched);
    let assets_read = plan.runs(Reader::CleanedAssets);
    let untouched = cleaned == document.html;
    let References {
        links,
        mut images,
        mut media,
        head,
    } = if raw_read || (assets_read && untouched) {
        references(document.html, document.base_url)
    } else {
        References::default()
    };
    if assets_read && !untouched {
        let after_rules = references(cleaned, document.base_url);
        (images, media) = (after_rules.images, after_rules.media);
    }
    // One tree of the document as a person sees it, for the formats a
    // tree reads.
    let tree = plan
        .runs(Reader::CleanedTree)
        .then(|| Html::parse_document(&seen));
    let tree = tree.as_ref();
    let formats = PageFormats {
        url: document.url.to_owned(),
        base_url: document.base_url.to_owned(),
        main_content: read_main.as_ref().map(|found| {
            if found.is_some() {
                MainContent::Found
            } else {
                MainContent::None
            }
        }),
        markdown: request.markdown.map(|format| match format.content() {
            MarkdownContent::Main => of_main(markdown),
            MarkdownContent::Full => markdown(&body(&seen)),
            MarkdownContent::Fit => main_html
                .map(|main| markdown(&fit(main, &seen, document.base_url)))
                .unwrap_or_default(),
        }),
        html: request.html.map(|format| match format.kind() {
            HtmlKind::Raw => document.serialization.as_content(document.html),
            HtmlKind::Cleaned => document.serialization.as_content(cleaned),
            HtmlKind::Main => main_html.map(ToOwned::to_owned).unwrap_or_default(),
        }),
        text: request.text.map(|Asked| of_main(text)),
        links: request.links.map(|Asked| links),
        images: request.images.map(|Asked| images),
        media: request.media.map(|Asked| media),
        tables: request.tables.and(tree).map(tables),
        metadata: request
            .metadata
            .and(tree)
            .map(|tree| metadata(head, tree, document.base_url)),
        contacts: request
            .contacts
            .and(tree)
            .map(|tree| contacts(tree, document.base_url)),
    };
    Measured {
        formats,
        main_content_length: read_main.map(|found| found.map_or(0, |(_, length)| length)),
    }
}

/// The document's metadata: its head's, read by the streaming pass, and
/// its structured data, read from `tree`, its canonical URL and microdata
/// URLs resolved against `base_url`.
fn metadata(head: Head, tree: &Html, base_url: &str) -> Metadata {
    let Head {
        title,
        description,
        canonical,
        lang,
        open_graph,
        twitter,
        meta,
    } = head;
    let canonical = canonical.and_then(|href| {
        Url::parse(base_url)
            .and_then(|base| base.join(&href))
            .ok()
            .map(String::from)
    });
    Metadata {
        title,
        description,
        canonical,
        lang,
        open_graph,
        twitter,
        json_ld: json_ld(tree),
        microdata: microdata(tree, base_url),
        meta,
    }
}

impl Serialization {
    /// `html`, serialized this way, as `content()` returns it.
    fn as_content(self, html: &str) -> String {
        match self {
            Self::Served => html.to_owned(),
            Self::Dom => light(html),
        }
    }
}

/// The length of the text of the main content `extractor` finds in
/// `document`, white space aside, in characters, its shadow roots' text
/// included; 0 when it has none: how a caller measures what
/// extraction-side removal took of it (ADR 0047).
pub fn main_content_length(
    document: &Document<'_>,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> usize {
    main_text_length(&composed(document.html), document.url, extractor, work)
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
        serialization: Serialization::Dom,
    };
    let request = FormatsRequest {
        markdown: Some(MarkdownFormat::Options(MarkdownOptions {
            content: MarkdownContent::Fit,
        })),
        html: Some(HtmlFormat::Asked(Asked)),
        text: Some(Asked),
        links: Some(Asked),
        images: Some(Asked),
        media: Some(Asked),
        tables: Some(Asked),
        metadata: Some(Asked),
        contacts: Some(Asked),
    };
    let _ = formats(&document, &request, &Context::default());
    let _ = light(&html);
}

#[cfg(test)]
mod tests;
