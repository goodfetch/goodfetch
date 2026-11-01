//! The `Extractor` interface (ADRs 0007 and 0033): what finds the part of
//! a document a person reads it for, and the two built-ins the setting
//! `content.extractor` chooses between, dom-smoothie (Readability's
//! algorithm, the default) and rs-trafilatura. The Extractor Plugin
//! adapter (area 7) implements the same interface, so a Plugin's main
//! content takes the built-ins' place and nothing else changes.

use dom_smoothie::Readability;

/// A main-content step: the document in, its main content out as HTML.
pub trait Extractor: Sync {
    /// The main content of `html`, the document of the page at `url`, as
    /// HTML; none when the step finds none. A document with no text outside
    /// its navigation landmarks never reaches it: the composition answers
    /// none for it first, whatever the step (pitfall D02).
    fn main_content(&self, html: &str, url: &str) -> Option<String>;
}

/// A built-in main-content step, by the value of `content.extractor` that
/// chooses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuiltIn {
    /// dom-smoothie, `dom_smoothie`: the default (ADR 0007).
    #[default]
    DomSmoothie,
    /// rs-trafilatura, `trafilatura`.
    Trafilatura,
}

impl BuiltIn {
    /// The built-in `content.extractor`'s value `name` chooses; none for a
    /// name no built-in has.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "dom_smoothie" => Some(Self::DomSmoothie),
            "trafilatura" => Some(Self::Trafilatura),
            _ => None,
        }
    }

    /// Its step.
    pub fn extractor(self) -> &'static dyn Extractor {
        match self {
            Self::DomSmoothie => &DomSmoothie,
            Self::Trafilatura => &Trafilatura,
        }
    }
}

/// dom-smoothie's step.
struct DomSmoothie;

impl Extractor for DomSmoothie {
    fn main_content(&self, html: &str, url: &str) -> Option<String> {
        dom_smoothie_article(html, url).map(|article| article.content.to_string())
    }
}

/// The article dom-smoothie reads from `html` at `url`. A URL the step
/// cannot resolve against, such as `about:blank`, is no reason to read
/// nothing: it only leaves relative URLs as they are.
pub(crate) fn dom_smoothie_article(html: &str, url: &str) -> Option<dom_smoothie::Article> {
    let mut readability = Readability::new(html, Some(url), None)
        .or_else(|_| Readability::new(html, None, None))
        .ok()?;
    readability.parse().ok()
}

/// rs-trafilatura's step, keeping what the Markdown converter writes:
/// tables, images, links and inline formatting. Its fallback is off, and
/// so are its least sizes: the fallback answers a short page's whole body
/// as one paragraph of run-together text, its headings, lists and tables
/// lost, and the least sizes send a short page to it.
struct Trafilatura;

impl Extractor for Trafilatura {
    fn main_content(&self, html: &str, url: &str) -> Option<String> {
        let options = rs_trafilatura::Options {
            include_tables: true,
            include_images: true,
            include_links: true,
            include_formatting: true,
            use_fallback_extraction: false,
            min_extracted_size: 0,
            min_output_size: 0,
            url: Some(url.to_owned()),
            ..rs_trafilatura::Options::default()
        };
        rs_trafilatura::extract_with_options(html, &options)
            .ok()?
            .content_html
    }
}
