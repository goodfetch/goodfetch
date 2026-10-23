//! The formats a request names, read from one document: the interface every
//! tier calls with the document it holds, and the composition of the
//! main-content step and the converters behind it.

use crate::main_content::{main_content, main_text_length};
use crate::markdown::markdown;
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

/// What a call reads with beside its document. Empty until the Clean-up
/// rules' cosmetic matcher (ADR 0047) and the `content.extractor` setting's
/// choice join it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {}

/// The formats `request` names, read from `document`, and no other: a
/// document with no main content answers each format empty and
/// `main_content: none`, never the whole page in its place.
pub fn formats(
    document: &Document<'_>,
    request: &FormatsRequest,
    context: &Context,
) -> PageFormats {
    let _ = context;
    let main = main_content(document.html, document.url);
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
/// aside, in characters; 0 when it has none: how a caller measures what
/// extraction-side removal took of it (ADR 0047).
pub fn main_content_length(document: &Document<'_>) -> usize {
    main_text_length(document.html, document.url)
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
