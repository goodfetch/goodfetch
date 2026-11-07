//! The `Extractor` interface (ADRs 0007 and 0033): what finds the part of
//! a document a person reads it for, and the two built-ins the setting
//! `content.extractor` chooses between, dom-smoothie (Readability's
//! algorithm, the default) and rs-trafilatura. The Extractor Plugin
//! adapter (area 7) implements the same interface, so a Plugin's main
//! content takes the built-ins' place and nothing else changes.
//!
//! A step takes the page's tree, never its text (ADR 0138), and parses
//! nothing of its own: dom-smoothie reads it through
//! `Readability::with_document`, rs-trafilatura through its vendored fork's
//! document entry (`vendor/rs-trafilatura`, #1916).

use std::fmt::{self, Debug, Formatter};

use dom_query::{NodeId, NodeRef};
use dom_smoothie::Readability;
use goodfetch_counted_work::ParsedHtml;

/// A main-content step: the document's tree in, which it consumes, its
/// main content out as a tree and as text.
pub trait Extractor: Sync {
    /// The main content of `document`, the tree of the page at `url`; none
    /// when the step finds none. A document with no text outside its
    /// navigation landmarks never reaches it: the composition answers none
    /// for it first, whatever the step (pitfall D02).
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found>;
}

/// The main content a step found: its tree, which the formats read (its
/// Markdown, text and "fit"), its HTML as the step wrote it, when it wrote
/// it, and its text.
pub struct Found {
    /// Its tree.
    pub(crate) article: Article,
    /// Its text, white space as the HTML holds it.
    pub(crate) text: String,
}

impl Found {
    /// The main content `html`, read from a tree of it: what a step that
    /// answers HTML alone answers, its HTML kept as it wrote it.
    #[must_use]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "ADR 0140: a raw parse of a step's answer, no page's; an Extractor Plugin's answer parses on its own row (F1.plugin_answer_parses)"
    )]
    pub fn of_html(html: String) -> Self {
        let tree = dom_query::Document::fragment(html.as_str());
        let root = tree.root();
        // A fragment's nodes are the children of the `<html>` its parse
        // puts them in.
        let root = root
            .element_children()
            .into_iter()
            .find(|child| child.has_name("html"))
            .unwrap_or(root)
            .id;
        let text = tree.root().text().to_string();
        Self {
            article: Article {
                tree,
                root,
                whole: false,
                html: Some(html),
            },
            text,
        }
    }

    /// Its text, white space as the HTML holds it.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Debug for Found {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Found")
            .field("text", &self.text)
            .finish_non_exhaustive()
    }
}

/// The tree a main content is read from: an element of a tree, and the
/// main content that element itself, or its children alone.
pub(crate) struct Article {
    /// The tree that holds it.
    pub(crate) tree: ParsedHtml,
    /// The element in it.
    pub(crate) root: NodeId,
    /// Whether the main content is that element, rather than its children.
    pub(crate) whole: bool,
    /// Its HTML as a step wrote it; none when it wrote none.
    pub(crate) html: Option<String>,
}

impl Article {
    /// The main content's tree, the copy of a page's tree the landmarks
    /// were taken out of: the children of `root`, its body.
    pub(crate) fn children_of(tree: ParsedHtml, root: NodeId) -> Self {
        Self {
            tree,
            root,
            whole: false,
            html: None,
        }
    }

    /// The element it is read from.
    pub(crate) fn root(&self) -> NodeRef<'_> {
        NodeRef::new(self.root, &self.tree.tree)
    }

    /// Its nodes, in order: its element, or the element's children.
    pub(crate) fn nodes(&self) -> Vec<NodeRef<'_>> {
        let root = self.root();
        if self.whole {
            vec![root]
        } else {
            root.children_it(false).collect()
        }
    }

    /// Its HTML: as the step wrote it, else its tree serialized.
    pub(crate) fn html(&self) -> String {
        match &self.html {
            Some(html) => html.clone(),
            None if self.whole => self.root().html().to_string(),
            None => self.root().inner_html().to_string(),
        }
    }
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

/// dom-smoothie's step, on the tree itself.
struct DomSmoothie;

impl Extractor for DomSmoothie {
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found> {
        // A URL the step cannot resolve against, such as `about:blank`, is
        // no reason to read nothing: it only leaves relative URLs as they
        // are. It is checked on an empty tree, so the page's own goes to the
        // one step that reads it.
        let url = Readability::with_document(ParsedHtml::default(), Some(url), None)
            .is_ok()
            .then_some(url);
        let mut readability = Readability::with_document(document, url, None).ok()?;
        let article = readability.parse().ok()?;
        // The step's answer is its element `#readability-page-1`, the first
        // of that ID, in the tree it leaves.
        let root = readability
            .doc
            .root()
            .descendants_it()
            .find(|node| node.is_element() && node.id_attr().as_deref() == Some(ARTICLE_ID))?
            .id;
        Some(Found {
            article: Article {
                tree: readability.doc,
                root,
                whole: true,
                html: Some(article.content.to_string()),
            },
            text: article.text_content.to_string(),
        })
    }
}

/// The ID of the element dom-smoothie answers its main content in.
const ARTICLE_ID: &str = "readability-page-1";

/// rs-trafilatura's step, keeping what the Markdown converter writes:
/// tables, images, links and inline formatting. Its fallback is off, and
/// so are its least sizes: the fallback answers a short page's whole body
/// as one paragraph of run-together text, its headings, lists and tables
/// lost, and the least sizes send a short page to it. It reads the tree
/// through its fork's document entry.
struct Trafilatura;

impl Extractor for Trafilatura {
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found> {
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
        rs_trafilatura::extract_document_with_options(document, &options)
            .ok()?
            .content_html
            .map(Found::of_html)
    }
}
