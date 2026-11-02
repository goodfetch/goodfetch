//! The main-content step (ADR 0007): the chosen [`Extractor`], dom-smoothie
//! by default, finds the part of a document a person reads it for and
//! answers it as HTML, or nothing when the document has none. A document
//! has none when
//! no text is left outside its navigation landmarks (its navigation, its
//! asides, and the header and footer of the page rather than of an
//! article): then the answer is empty, never the page's chrome, which
//! Readability falls back to, in its place (pitfall D02).

use goodfetch_counted_work::{CountedWork, Row};

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
    work: &CountedWork,
) -> Option<(String, usize)> {
    read(html, url, extractor, work)
}

/// The length of the text of the main content `extractor` finds in `html`,
/// read at `url`, white space aside, in characters; 0 when it has none.
pub(crate) fn main_text_length(
    html: &str,
    url: &str,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> usize {
    read(html, url, extractor, work).map_or(0, |(_, length)| length)
}

/// The text of `html`'s main content, read at `url`, and its title, each
/// with every run of white space made one space and none at either end;
/// an empty text when it has no main content.
pub(crate) fn title_and_main_text(html: &str, url: &str, work: &CountedWork) -> (String, String) {
    let title = collapsed(
        &work
            .parse_html(Row::PageParses, html)
            .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "head title"))
            .text(),
    );
    let text = has_content_text(html, work)
        .then(|| {
            work.count(Row::PageReadabilityRuns);
            dom_smoothie_article(html, url)
        })
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
fn read(
    html: &str,
    url: &str,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> Option<(String, usize)> {
    let remaining = outside_landmarks(html, work)?;
    work.count(Row::PageReadabilityRuns);
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
#[expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
fn text_length(html: &str) -> usize {
    visible(&dom_query::Document::fragment(html).root().text())
        .chars()
        .count()
}

/// Whether `html` holds text outside its navigation landmarks.
fn has_content_text(html: &str, work: &CountedWork) -> bool {
    outside_landmarks(html, work).is_some()
}

/// The HTML of `html`'s body without its navigation landmarks and what no
/// person reads; none when no text is left in it.
fn outside_landmarks(html: &str, work: &CountedWork) -> Option<String> {
    let document = work.parse_html(Row::PageParses, html);
    document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, NOT_CONTENT))
        .remove();
    for landmark in document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "header, footer"))
        .nodes()
    {
        let sectioning = work.literal_selector(Row::PageSelectorCompilations, SECTIONING);
        if !landmark
            .ancestors_it(None)
            .any(|ancestor| ancestor.is_match(&sectioning))
        {
            landmark.remove_from_parent();
        }
    }
    let body =
        document.select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "body"));
    (!visible(&body.text()).is_empty()).then(|| body.inner_html().to_string())
}

/// `text` without its white space.
fn visible(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
