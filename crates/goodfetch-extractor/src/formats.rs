//! The formats a request names, read from one document: the interface every
//! tier calls with the document it holds, and the composition of the
//! main-content step and the converters behind it.

use std::fmt::{self, Debug, Formatter};

use crate::blocks::{PageLists, fit, text};
use crate::contacts::contacts;
use crate::extractor::{Article, BuiltIn, Extractor};
use crate::light::light;
use crate::main_content::{main_content, main_text_length};
use crate::markdown::markdown;
use crate::plan::{Demand, Plan, Reader};
use crate::references::{Head, References, assets, references};
use crate::shadow::compose_in_place;
use crate::structured_data::{json_ld, microdata};
use crate::tables::tables;
use dom_query::NodeRef;
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_protocol::page::{
    Asked, Contacts, FormatsRequest, HtmlFormat, HtmlKind, MainContent, MarkdownContent,
    MarkdownFormat, MarkdownOptions, Metadata, MicrodataItem, PageFormats, Table,
};
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
/// call's [`Context`] carries: it takes out of a document's tree, in place,
/// the elements the cosmetic rules match, inside its shadow roots too (each
/// a `<template shadowrootmode>`).
pub trait CleanUp {
    /// Removes from `document`, the tree of the page at `url`, the elements
    /// the cosmetic rules match; whether it removed any.
    fn remove(&self, document: &ParsedHtml, url: &str) -> bool;
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
///
/// Every format is read from at most one tree of the page (ADR 0138), its
/// readers in their order: those of the page as it stands (`html.raw`,
/// links, the head), streamed; the clean-up, in place; the readers of the
/// cleaned page as it stands (`html.cleaned`, images and media); its shadow
/// roots composed, in place; the readers of the whole page as a person
/// sees it (full Markdown, tables, structured data, contacts, the lists
/// "fit" compares); then the main-content step, which consumes the tree;
/// then the readers of its answer's tree (`html.main`, text, Markdown and
/// "fit"). The tree is serialized only for `html.cleaned`, and only when
/// the rules changed it.
pub fn measured_formats(
    document: &Document<'_>,
    request: &FormatsRequest,
    context: &Context<'_>,
) -> Measured {
    measured(document, request, context, || {
        context.work.parse_html(Row::PageParses, document.html)
    })
}

/// [`measured_formats`] of `document` whose untouched tree, `kept`, a
/// caller keeps for other reads (a handler's page, spec #1577's #1918):
/// the formats that read a tree read a copy of it, on `F1.tree_copies`,
/// never a parse of the HTML, and `kept` stays as it was.
pub fn measured_formats_of(
    document: &Document<'_>,
    kept: &ParsedHtml,
    request: &FormatsRequest,
    context: &Context<'_>,
) -> Measured {
    measured(document, request, context, || {
        context.work.copy_tree(Row::PageTreeCopies, kept)
    })
}

/// [`measured_formats`], its one tree, when a reader reads one or the
/// rules run, from `tree_of`.
fn measured(
    document: &Document<'_>,
    request: &FormatsRequest,
    context: &Context<'_>,
    tree_of: impl FnOnce() -> ParsedHtml,
) -> Measured {
    let work = &context.work;
    // The demand plan runs only the readers the request reads from (ADR
    // 0138). A request that reads nothing of the cleaned page runs no rule.
    let plan = Plan::of(&Demand {
        formats: request,
        learning: false,
    });
    let clean_up = context.clean_up.filter(|_| plan.runs(Reader::CleanUp));
    let full_markdown = request
        .markdown
        .is_some_and(|format| format.content() == MarkdownContent::Full);
    // One tree of the page, when a reader reads one or the rules run.
    let mut tree = (clean_up.is_some()
        || full_markdown
        || plan.runs(Reader::CleanedTree)
        || plan.runs(Reader::MainContent))
    .then(tree_of);
    let removed = match (clean_up, &tree) {
        (Some(clean_up), Some(tree)) => clean_up.remove(tree, document.url),
        _ => false,
    };
    let (cleaned, references) = as_it_stands(document, request, plan, tree.as_ref(), removed);
    let References {
        links,
        images,
        media,
        head,
    } = references;
    // The page as a person sees it: its shadow roots composed, in place, and
    // its text nodes merged as a parse of it would hold them.
    if let Some(tree) = &tree
        && (compose_in_place(tree, work) || removed)
    {
        merge_texts(tree);
    }
    let Whole {
        full,
        tables,
        structured,
        contacts,
        lists,
    } = tree
        .as_ref()
        .map(|tree| Whole::read(tree, request, document.base_url, work))
        .unwrap_or_default();
    // The main-content step runs only for a format read from its answer
    // (story 8): outside, none read; inside, none found. It consumes the
    // tree, its last reader.
    let read_main = plan
        .runs(Reader::MainContent)
        .then(|| tree.take())
        .flatten()
        .map(|tree| main_content(tree, document.url, context.extractor, work));
    let main = read_main
        .as_ref()
        .and_then(Option::as_ref)
        .map(|(article, _)| article);
    // Its readers, "fit" last, as it takes blocks out of its tree.
    let main_html = request
        .html
        .filter(|format| format.kind() == HtmlKind::Main)
        .map(|_| main.map(Article::html).unwrap_or_default());
    let main_text = request.text.map(|Asked| {
        main.map(|article| text(article.nodes()))
            .unwrap_or_default()
    });
    let markdown = request.markdown.map(|format| match format.content() {
        MarkdownContent::Main => main
            .map(|article| markdown(article.nodes()))
            .unwrap_or_default(),
        MarkdownContent::Full => full.unwrap_or_default(),
        MarkdownContent::Fit => match (main, &lists) {
            (Some(article), Some(lists)) if fit(article, lists, work) => markdown(article.nodes()),
            _ => String::new(),
        },
    });
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
        markdown,
        html: request.html.map(|format| match format.kind() {
            HtmlKind::Raw => document.serialization.as_content(document.html),
            HtmlKind::Cleaned => document
                .serialization
                .as_content(cleaned.as_deref().unwrap_or(document.html)),
            HtmlKind::Main => main_html.unwrap_or_default(),
        }),
        text: main_text,
        links: request.links.map(|Asked| links),
        images: request.images.map(|Asked| images),
        media: request.media.map(|Asked| media),
        tables,
        metadata: structured
            .map(|(json_ld, microdata)| metadata(head, json_ld, microdata, document.base_url)),
        contacts,
    };
    Measured {
        formats,
        main_content_length: read_main.map(|found| found.map_or(0, |(_, length)| length)),
    }
}

/// What the readers of the page as it stands read: `html.cleaned`, its
/// serialization when the rules changed its tree, and the streaming pass's
/// links and head of the page as served, with the images and media of the
/// cleaned page, its shadow roots' templates included. When the rules
/// removed nothing the cleaned page is the page as served, which the one
/// streaming pass reads; else its images and media are read from its tree.
fn as_it_stands(
    document: &Document<'_>,
    request: &FormatsRequest,
    plan: Plan,
    tree: Option<&ParsedHtml>,
    removed: bool,
) -> (Option<String>, References) {
    let cleaned = tree
        .filter(|_| {
            removed
                && request
                    .html
                    .is_some_and(|format| format.kind() == HtmlKind::Cleaned)
        })
        .map(|tree| tree.html().to_string());
    let assets_read = plan.runs(Reader::CleanedAssets);
    let mut references = if plan.runs(Reader::Untouched) || (assets_read && !removed) {
        references(document.html, document.base_url)
    } else {
        References::default()
    };
    if let Some(tree) = tree.filter(|_| assets_read && removed) {
        (references.images, references.media) = assets(tree, document.base_url);
    }
    (cleaned, references)
}

/// What the readers of the whole page as a person sees it read, before
/// the main-content step consumes its tree.
#[derive(Default)]
struct Whole {
    /// Its Markdown, of its whole body.
    full: Option<String>,
    tables: Option<Vec<Table>>,
    /// Its JSON-LD and microdata.
    structured: Option<(Vec<serde_json::Value>, Vec<MicrodataItem>)>,
    contacts: Option<Contacts>,
    /// The lists "fit" compares the main content's to.
    lists: Option<PageLists>,
}

impl Whole {
    /// What `request` reads of `tree`, the page whose base URL is
    /// `base_url`.
    fn read(
        tree: &ParsedHtml,
        request: &FormatsRequest,
        base_url: &str,
        work: &CountedWork,
    ) -> Self {
        let asks = |content| {
            request
                .markdown
                .is_some_and(|format| format.content() == content)
        };
        Self {
            full: asks(MarkdownContent::Full).then(|| {
                tree.body()
                    .map(|body| markdown(body.children_it(false)))
                    .unwrap_or_default()
            }),
            tables: request.tables.map(|Asked| tables(tree, work)),
            structured: request
                .metadata
                .map(|Asked| (json_ld(tree), microdata(tree, base_url))),
            contacts: request.contacts.map(|Asked| contacts(tree, base_url)),
            lists: asks(MarkdownContent::Fit).then(|| PageLists::of(tree, base_url, work)),
        }
    }
}

/// Merges each run of adjacent text nodes of `tree` into one, as a parse of
/// its serialization holds them, once a removal or a composition left some
/// side by side; by hand, as a page's tree can be deeper than a thread's
/// stack allows a recursion.
fn merge_texts(tree: &ParsedHtml) {
    let mut next = vec![tree.root()];
    while let Some(node) = next.pop() {
        let mut previous: Option<NodeRef<'_>> = None;
        for child in node.children_it(false).collect::<Vec<_>>() {
            if child.is_text() {
                if let Some(text) = previous.filter(NodeRef::is_text) {
                    let joined = format!("{}{}", text.text(), child.text());
                    text.set_text(joined);
                    child.remove_from_parent();
                    continue;
                }
            } else if child.is_element() {
                next.push(child);
            }
            previous = Some(child);
        }
    }
}

/// The document's metadata: its head's, read by the streaming pass, and
/// its structured data, read from its tree, its canonical URL resolved
/// against `base_url`.
fn metadata(
    head: Head,
    json_ld: Vec<serde_json::Value>,
    microdata: Vec<MicrodataItem>,
    base_url: &str,
) -> Metadata {
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
        json_ld,
        microdata,
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
/// included; 0 when it has none: how a caller measures exactly what
/// extraction-side removal took of it (ADR 0047's share under
/// `extraction.removal_share: exact`), at the cost of one parse and one run
/// of the step, on `F1.share_readability_runs`.
pub fn main_content_length(
    document: &Document<'_>,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> usize {
    main_length(
        work.parse_html(Row::PageParses, document.html),
        document,
        extractor,
        work,
    )
}

/// [`main_content_length`] of `document` whose untouched tree, `kept`, a
/// caller keeps for other reads: it reads a copy of it, on
/// `F1.tree_copies`, never a parse of the HTML.
pub fn main_content_length_of(
    document: &Document<'_>,
    kept: &ParsedHtml,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> usize {
    main_length(
        work.copy_tree(Row::PageTreeCopies, kept),
        document,
        extractor,
        work,
    )
}

/// [`main_content_length`] read on `tree`, the page's tree, which it
/// consumes.
fn main_length(
    tree: ParsedHtml,
    document: &Document<'_>,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> usize {
    if compose_in_place(&tree, work) {
        merge_texts(&tree);
    }
    main_text_length(tree, document.url, extractor, work)
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
