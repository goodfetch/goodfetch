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
    read(html, url).map(|(content, _)| content)
}

/// The length of the text of `html`'s main content, read at `url`, white
/// space aside, in characters; 0 when it has none.
pub(crate) fn main_text_length(html: &str, url: &str) -> usize {
    read(html, url).map_or(0, |(_, length)| length)
}

/// The text of `html`'s main content, read at `url`, and its title, each
/// with every run of white space made one space and none at either end;
/// an empty text when it has no main content.
pub(crate) fn title_and_main_text(html: &str, url: &str) -> (String, String) {
    let title = collapsed(&Document::from(html).select("head title").text());
    let text = parsed(html, url)
        .map(|article| collapsed(&article.text_content))
        .unwrap_or_default();
    (title, text)
}

/// `text` with each run of white space made one space, none at either end.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The main content of `html`, read at `url`, as HTML, and the length of
/// its text.
fn read(html: &str, url: &str) -> Option<(String, usize)> {
    let article = parsed(html, url)?;
    let length = visible(&article.text_content).chars().count();
    if length == 0 {
        return None;
    }
    Some((article.content.to_string(), length))
}

/// The article dom-smoothie reads from `html` at `url`; none when the
/// document holds no text outside its navigation landmarks.
fn parsed(html: &str, url: &str) -> Option<dom_smoothie::Article> {
    if !has_content_text(html) {
        return None;
    }
    // A URL the step cannot resolve against, such as `about:blank`, is no
    // reason to read nothing: it only leaves relative URLs as they are.
    let mut readability = Readability::new(html, Some(url), None)
        .or_else(|_| Readability::new(html, None, None))
        .ok()?;
    readability.parse().ok()
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
