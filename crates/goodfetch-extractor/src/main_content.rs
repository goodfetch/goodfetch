//! The main-content step (ADR 0007): dom-smoothie, Readability's algorithm,
//! finds the part of a document a person reads it for and answers it as
//! HTML, or nothing when the document has none. A document has none when
//! no text is left outside its navigation landmarks (its navigation, its
//! asides, and the header and footer of the page rather than of an
//! article): then the answer is empty, never the page's chrome, which
//! Readability falls back to, in its place (pitfall D02).

use dom_query::Document;
use dom_smoothie::Readability;

/// What holds no main content: navigation landmarks, and what no person
/// reads.
const NOT_CONTENT: &str = "script, style, noscript, template, [hidden], nav, aside, \
     [role=navigation], [role=banner], [role=contentinfo], [role=complementary]";

/// The sectioning elements whose `<header>` and `<footer>` are their own,
/// not the page's banner and content information (WAI-ARIA's HTML mapping).
const SECTIONING: &str = "article, aside, main, nav, section";

/// The main content of `html`, read at `url`, as HTML; none when the
/// document has no text a person reads it for.
pub(crate) fn main_content(html: &str, url: &str) -> Option<String> {
    if !has_content_text(html) {
        return None;
    }
    // A URL the step cannot resolve against, such as `about:blank`, is no
    // reason to read nothing: it only leaves relative URLs as they are.
    let mut readability = Readability::new(html, Some(url), None)
        .or_else(|_| Readability::new(html, None, None))
        .ok()?;
    let article = readability.parse().ok()?;
    if visible(&article.text_content).is_empty() {
        return None;
    }
    Some(article.content.to_string())
}

/// Whether `html` holds text outside its navigation landmarks.
fn has_content_text(html: &str) -> bool {
    let document = Document::from(html);
    document.select(NOT_CONTENT).remove();
    for landmark in document.select("header, footer").nodes() {
        if !landmark
            .ancestors_it(None)
            .any(|ancestor| ancestor.is(SECTIONING))
        {
            landmark.remove_from_parent();
        }
    }
    !visible(&document.select("body").text()).is_empty()
}

/// `text` without its white space.
fn visible(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
