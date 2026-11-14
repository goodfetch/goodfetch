//! The main-content step (ADR 0007): the chosen [`Extractor`], dom-smoothie
//! by default, finds the part of a document a person reads it for and
//! answers it as HTML, or nothing when the document has none. A document
//! has none when
//! no text is left outside its navigation landmarks (its navigation, its
//! asides, and the header and footer of the page rather than of an
//! article): then the answer is empty, never the page's chrome, which
//! Readability falls back to, in its place (pitfall D02).
//!
//! The step reads the page's tree and consumes it (ADR 0138): the text
//! outside the landmarks is read on the tree before it, which removes
//! nothing, and the fallback's HTML, when a reader needs it, from the tree
//! as the step was handed it, which a built-in keeps and any other step
//! copies first (#2639).

use dom_query::{Matcher, NodeRef};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

use crate::extractor::{Article, Extractor, Found};
use crate::whitespace;

/// What holds no main content: navigation landmarks, and what no person
/// reads.
const NOT_CONTENT: &str = "script, style, noscript, template, [hidden], nav, aside, \
     [role=navigation], [role=banner], [role=contentinfo], [role=complementary]";

/// The sectioning elements whose `<header>` and `<footer>` are their own,
/// not the page's banner and content information (WAI-ARIA's HTML mapping).
const SECTIONING: &str = "article, aside, main, nav, section";

/// Every selector the step matches, which [`compile_selectors`] keeps.
const SELECTORS: [&str; 5] = [
    "head title",
    NOT_CONTENT,
    "header, footer",
    SECTIONING,
    "body",
];

/// Compiles into the engine's cache on `work` every selector the step
/// matches, so that a Run which runs it compiles them before its first page,
/// not on it (ADRs 0138 and 0140).
pub fn compile_selectors(work: &CountedWork) {
    for selector in SELECTORS {
        let _ = work.literal_selector(Row::PageSelectorCompilations, selector);
    }
}

/// The main content `extractor` finds in `tree`, the page at `url`, which
/// it consumes, as the tree it is read from, and the length of its text, as
/// [`main_text_length`] counts it; none when the document has no text a
/// person reads it for. A step that finds no text where the document holds
/// some answers what is left of its body outside those landmarks, read
/// from the tree as the step was handed it
/// ([`Extractor::main_content_or_document`]): a document with text a
/// person reads is never answered as one without (pitfall D02).
pub(crate) fn main_content(
    tree: ParsedHtml,
    url: &str,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> Option<(Article, usize)> {
    let remaining = text_outside_landmarks(&tree, work);
    if length(&remaining) == 0 {
        return None;
    }
    work.count(Row::PageReadabilityRuns);
    match extractor.main_content_or_document(tree, url, work) {
        Ok(Found { article, text }) => Some((article, length(&text))),
        Err(document) => Some((outside_landmarks(document, work), length(&remaining))),
    }
}

/// The length of the text of the main content `extractor` finds in `tree`,
/// the page at `url`, which it consumes, white space aside, in characters;
/// 0 when it has none: ADR 0047's exact share, its run counted on
/// `F1.share_readability_runs`, apart from the formats' own.
pub(crate) fn main_text_length(
    tree: ParsedHtml,
    url: &str,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> usize {
    let remaining = text_outside_landmarks(&tree, work);
    if length(&remaining) == 0 {
        return 0;
    }
    work.count(Row::PageShareReadabilityRuns);
    extractor
        .main_content(tree, url)
        .map(|found| length(&found.text))
        .filter(|found| *found > 0)
        .unwrap_or_else(|| length(&remaining))
}

/// The text of `tree`'s main content, the page at `url`, found by
/// `extractor`, with every run of white space made one space and none at
/// either end, and its title, collapsed as `metadata.title` is (ASCII
/// white space alone, as the HTML Standard reads a title); an empty text
/// when it has no main content. The main-content step consumes the tree, the last reader of the
/// page as it was served (ADR 0138), with its fallback as the formats read
/// it, and no copy: the fallback's text is read before it.
pub(crate) fn title_and_main_text_of(
    tree: ParsedHtml,
    url: &str,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> (String, String) {
    let title = whitespace::collapsed(
        &tree
            .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "head title"))
            .text(),
    );
    let remaining = text_outside_landmarks(&tree, work);
    if length(&remaining) == 0 {
        return (title, String::new());
    }
    work.count(Row::PageReadabilityRuns);
    let text = extractor
        .main_content(tree, url)
        .map(|found| found.text)
        .filter(|text| length(text) > 0)
        .unwrap_or(remaining);
    (title, spaced(&text))
}

/// `text` with each run of white space, Unicode's included, made one
/// space, none at either end: a reader sees a no-break space in a page's
/// text as any other.
fn spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The length of `text`, white space aside, in characters.
fn length(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

/// The text of `tree`'s body outside its navigation landmarks and what no
/// person reads, in document order, read without removing anything.
fn text_outside_landmarks(tree: &ParsedHtml, work: &CountedWork) -> String {
    let not_content = work.literal_selector(Row::PageSelectorCompilations, NOT_CONTENT);
    let landmarks = work.literal_selector(Row::PageSelectorCompilations, "header, footer");
    let sectioning = work.literal_selector(Row::PageSelectorCompilations, SECTIONING);
    let body = tree.select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "body"));
    let mut text = String::new();
    // Depth first, by hand: a page's tree can be deeper than a thread's
    // stack allows a recursion.
    let mut next: Vec<NodeRef<'_>> = body.nodes().iter().rev().copied().collect();
    while let Some(node) = next.pop() {
        if node.is_text() {
            text.push_str(&node.text());
        } else if node.is_element() && !is_landmark(&node, (&not_content, &landmarks, &sectioning))
        {
            next.extend(node.children_it(true));
        }
    }
    text
}

/// Whether `node` is what [`outside_landmarks`] removes: what holds no main
/// content, or a header or footer of the page, not of a sectioning element.
fn is_landmark(
    node: &NodeRef<'_>,
    (not_content, landmarks, sectioning): (&Matcher, &Matcher, &Matcher),
) -> bool {
    node.is_match(not_content)
        || (node.is_match(landmarks)
            && !node
                .ancestors_it(None)
                .any(|ancestor| ancestor.is_match(sectioning)))
}

/// `document`'s body without its navigation landmarks and what no person
/// reads, which it removes from the tree it takes.
fn outside_landmarks(document: ParsedHtml, work: &CountedWork) -> Article {
    document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, NOT_CONTENT))
        .remove();
    let sectioning = work.literal_selector(Row::PageSelectorCompilations, SECTIONING);
    for landmark in document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "header, footer"))
        .nodes()
    {
        if !landmark
            .ancestors_it(None)
            .any(|ancestor| ancestor.is_match(&sectioning))
        {
            landmark.remove_from_parent();
        }
    }
    let body = document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "body"))
        .nodes()
        .first()
        .map_or_else(|| document.root().id, |body| body.id);
    Article::children_of(document, body)
}
