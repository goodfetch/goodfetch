//! The `Extractor` interface (ADRs 0007 and 0033): what finds the part of
//! a document a person reads it for, and the two built-ins the setting
//! `content.extractor` chooses between, dom-smoothie (Readability's
//! algorithm, the default) and rs-trafilatura, with the number of
//! attempts `extraction.readability` gives dom-smoothie. The Extractor Plugin
//! adapter (area 7) implements the same interface, so a Plugin's main
//! content takes the built-ins' place and nothing else changes.
//!
//! A step takes the page's tree, never its text (ADR 0138), and parses
//! nothing of its own: dom-smoothie reads it through
//! `Readability::with_document`, rs-trafilatura through its vendored fork's
//! document entry (`vendor/rs-trafilatura`, #1916). Both keep the tree as
//! they were handed it, which the composition reads its landmarks fallback
//! from when they find nothing, so it copies no tree for it (#2639).

use std::fmt::{self, Debug, Formatter};

use dom_query::{NodeId, NodeRef};
use dom_smoothie::{Kept, ParsePolicy, Readability};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

/// A main-content step: the document's tree in, which it consumes, its
/// main content out as a tree and as text.
pub trait Extractor: Sync {
    /// The main content of `document`, the tree of the page at `url`; none
    /// when the step finds none. A document with no text outside its
    /// navigation landmarks never reaches it: the composition answers none
    /// for it first, whatever the step (pitfall D02).
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found>;

    /// The main content of `document`, the tree of the page at `url`, as
    /// [`Extractor::main_content`] finds it, when it has text; else
    /// `document` as it was handed, which the composition reads its
    /// landmarks fallback from (pitfall D02). A step that consumes the tree
    /// keeps none of it, so this copies it first, on `work`'s
    /// `F1.tree_copies`; a step that keeps the tree it was handed answers
    /// it with no copy (#2639).
    ///
    /// # Errors
    ///
    /// `document` as it was handed, when the step finds no main content
    /// with text in it.
    fn main_content_or_document(
        &self,
        document: ParsedHtml,
        url: &str,
        work: &CountedWork,
    ) -> Result<Found, ParsedHtml> {
        found_or_copy(self, document, url, work)
    }
}

/// The main content `extractor` finds in `document`, when it has text;
/// else the copy of `document` taken before the step consumed it, on
/// `work`'s `F1.tree_copies`.
fn found_or_copy<E: Extractor + ?Sized>(
    extractor: &E,
    document: ParsedHtml,
    url: &str,
    work: &CountedWork,
) -> Result<Found, ParsedHtml> {
    let copy = work.copy_tree(Row::PageTreeCopies, &document);
    extractor
        .main_content(document, url)
        .filter(Found::has_text)
        .ok_or(copy)
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

    /// Whether its text holds anything but white space.
    fn has_text(&self) -> bool {
        self.text.chars().any(|c| !c.is_whitespace())
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
}

/// How many times dom-smoothie's step may try a page, by the value of
/// `extraction.readability` that chooses it (spec #2164).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attempts {
    /// `retry`, the default: dom-smoothie's `parse()`, up to four
    /// attempts while one finds under 500 characters, each on a copy of
    /// the tree, the longest kept.
    #[default]
    Retry,
    /// `single_attempt`: the first of those attempts alone, on the tree
    /// itself.
    SingleAttempt,
}

impl Attempts {
    /// The value `extraction.readability`'s `name` chooses; none for a
    /// name it has no value of.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "retry" => Some(Self::Retry),
            "single_attempt" => Some(Self::SingleAttempt),
            _ => None,
        }
    }
}

/// A main-content step as a page's settings choose it: the built-in
/// `content.extractor` names, and the attempts `extraction.readability`
/// gives it, which rs-trafilatura's one pass does not read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chosen {
    /// The built-in.
    pub built_in: BuiltIn,
    /// The attempts dom-smoothie may make.
    pub attempts: Attempts,
}

impl Chosen {
    /// Its step.
    #[must_use]
    pub fn extractor(self) -> &'static dyn Extractor {
        match (self.built_in, self.attempts) {
            (BuiltIn::DomSmoothie, Attempts::Retry) => &DomSmoothie(Attempts::Retry),
            (BuiltIn::DomSmoothie, Attempts::SingleAttempt) => {
                &DomSmoothie(Attempts::SingleAttempt)
            }
            (BuiltIn::Trafilatura, _) => &Trafilatura,
        }
    }
}

/// dom-smoothie's step, on the tree itself, making the attempts it holds.
struct DomSmoothie(Attempts);

impl DomSmoothie {
    /// The step on `document`, the tree of the page at `url`, which it
    /// reads through: none when the URL is one it cannot resolve against,
    /// such as `about:blank`, which is no reason to read nothing; it only
    /// leaves relative URLs as they are. The URL is checked on an empty
    /// tree, so the page's own goes to the one step that reads it.
    fn readability(document: ParsedHtml, url: &str) -> Readability {
        let url = Readability::with_document(ParsedHtml::default(), Some(url), None)
            .is_ok()
            .then_some(url);
        Readability::with_document(document, url, None)
            .expect("invariant: the URL is none or one an empty tree was read with")
    }
}

impl Extractor for DomSmoothie {
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found> {
        let mut readability = Self::readability(document, url);
        let article = match self.0 {
            Attempts::Retry => readability.parse(),
            // The first attempt `parse()` makes, with every flag on.
            Attempts::SingleAttempt => readability.parse_with_policy(ParsePolicy::Strict),
        }
        .ok()?;
        found_in(readability.doc, &article)
    }

    /// Its retries read a copy of the tree each and keep the tree itself,
    /// which answers the fallback with no copy; its single attempt reads
    /// the tree itself, so the fallback reads a copy taken before it.
    fn main_content_or_document(
        &self,
        document: ParsedHtml,
        url: &str,
        work: &CountedWork,
    ) -> Result<Found, ParsedHtml> {
        match self.0 {
            Attempts::Retry => {
                let Kept { untouched, parsed } = Self::readability(document, url).parse_keeping();
                parsed
                    .ok()
                    .and_then(|(article, tree)| found_in(tree, &article))
                    .filter(Found::has_text)
                    .ok_or(untouched)
            }
            Attempts::SingleAttempt => found_or_copy(self, document, url, work),
        }
    }
}

/// The main content dom-smoothie found, `article`, in `tree`, the tree it
/// left: its element `#readability-page-1`, the first of that ID.
fn found_in(tree: ParsedHtml, article: &dom_smoothie::Article) -> Option<Found> {
    let root = tree
        .root()
        .descendants_it()
        .find(|node| node.is_element() && node.id_attr().as_deref() == Some(ARTICLE_ID))?
        .id;
    Some(Found {
        article: Article {
            tree,
            root,
            whole: true,
            html: Some(article.content.to_string()),
        },
        text: article.text_content.to_string(),
    })
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

impl Trafilatura {
    /// Its options for the page at `url`.
    fn options(url: &str) -> rs_trafilatura::Options {
        rs_trafilatura::Options {
            include_tables: true,
            include_images: true,
            include_links: true,
            include_formatting: true,
            use_fallback_extraction: false,
            min_extracted_size: 0,
            min_output_size: 0,
            url: Some(url.to_owned()),
            ..rs_trafilatura::Options::default()
        }
    }
}

impl Extractor for Trafilatura {
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found> {
        rs_trafilatura::extract_document_with_options(document, &Self::options(url))
            .ok()?
            .content_html
            .map(Found::of_html)
    }

    /// It keeps the tree as it was handed, its backup from before its
    /// cleaning, which answers the fallback with no copy.
    fn main_content_or_document(
        &self,
        document: ParsedHtml,
        url: &str,
        _work: &CountedWork,
    ) -> Result<Found, ParsedHtml> {
        let (untouched, extracted) =
            rs_trafilatura::extract_document_keeping_with_options(document, &Self::options(url));
        extracted
            .ok()
            .and_then(|extracted| extracted.content_html)
            .map(Found::of_html)
            .filter(Found::has_text)
            .ok_or(untouched)
    }
}
