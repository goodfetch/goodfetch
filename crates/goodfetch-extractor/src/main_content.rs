//! The main-content step (ADR 0007): the chosen [`Extractor`], dom-smoothie
//! by default, finds the part of a document a person reads it for and
//! answers it as HTML, or nothing when the document has none. A document
//! has none when
//! no text is left outside its navigation landmarks (its navigation, its
//! asides, and the header and footer of the page rather than of an
//! article): then the answer is empty, never the page's chrome, which
//! Readability falls back to, in its place (pitfall D02).

use dom_query::Document;

use crate::extractor::{Extractor, dom_smoothie_article};

/// What holds no main content: navigation landmarks, and what no person
/// reads.
const NOT_CONTENT: &str = "script, style, noscript, template, [hidden], nav, aside, \
     [role=navigation], [role=banner], [role=contentinfo], [role=complementary]";

/// The sectioning elements whose `<header>` and `<footer>` are their own,
/// not the page's banner and content information (WAI-ARIA's HTML mapping).
const SECTIONING: &str = "article, aside, main, nav, section";

/// The main content `extractor` finds in `html`, read at `url`, as HTML,
/// and the length of its text, as [`main_text_length`] counts it; none
/// when the document has no text a person reads it for.
pub(crate) fn main_content(
    html: &str,
    url: &str,
    extractor: &dyn Extractor,
) -> Option<(String, usize)> {
    read(html, url, extractor)
}

/// The length of the text of the main content `extractor` finds in `html`,
/// read at `url`, white space aside, in characters; 0 when it has none.
pub(crate) fn main_text_length(html: &str, url: &str, extractor: &dyn Extractor) -> usize {
    read(html, url, extractor).map_or(0, |(_, length)| length)
}

/// The text of `html`'s main content, read at `url`, and its title, each
/// with every run of white space made one space and none at either end;
/// an empty text when it has no main content.
pub(crate) fn title_and_main_text(html: &str, url: &str) -> (String, String) {
    let title = collapsed(&Document::from(html).select("head title").text());
    let text = has_content_text(html)
        .then(|| dom_smoothie_article(html, url))
        .flatten()
        .map(|article| collapsed(&article.text_content))
        .unwrap_or_default();
    (title, text)
}

/// `text` with each run of white space made one space, none at either end.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The main content `extractor` finds in `html`, read at `url`, as HTML,
/// and the length of its text; none when the document holds no text
/// outside its navigation landmarks. A step that finds no text where the
/// document holds some answers what is left of its body outside those
/// landmarks: a document with text a person reads is never answered as
/// one without (pitfall D02).
fn read(html: &str, url: &str, extractor: &dyn Extractor) -> Option<(String, usize)> {
    let remaining = outside_landmarks(html)?;
    let found = extractor.main_content(html, url).map(|content| {
        let length = text_length(&content);
        (content, length)
    });
    match found {
        Some(found) if found.1 > 0 => Some(found),
        Some(_) | None => {
            let length = text_length(&remaining);
            Some((remaining, length))
        }
    }
}

/// The length of the text of the HTML fragment `html`, white space aside.
fn text_length(html: &str) -> usize {
    visible(&Document::fragment(html).root().text())
        .chars()
        .count()
}

/// Whether `html` holds text outside its navigation landmarks.
fn has_content_text(html: &str) -> bool {
    outside_landmarks(html).is_some()
}

/// The HTML of `html`'s body without its navigation landmarks and what no
/// person reads; none when no text is left in it.
fn outside_landmarks(html: &str) -> Option<String> {
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
    let body = document.select("body");
    (!visible(&body.text()).is_empty()).then(|| body.inner_html().to_string())
}

/// `text` without its white space.
fn visible(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
