//! The main-content step `content.extractor` chooses (#788): each
//! built-in by its name, the main content each finds where they differ,
//! and the rules the composition holds every step to, whichever it is.

use goodfetch_protocol::page::{FormatsRequest, MainContent, PageFormats};
use rstest::rstest;
use serde_json::json;

use super::URL;
use crate::{BuiltIn, Context, Document, Serialization, formats};

/// The Markdown and the text `built_in` reads from `html` at [`URL`].
fn read_with(html: &str, built_in: BuiltIn) -> PageFormats {
    let request: FormatsRequest =
        serde_json::from_value(json!({ "markdown": true, "text": true })).unwrap();
    formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Served,
        },
        &request,
        &Context {
            clean_up: None,
            extractor: built_in.extractor(),
            work: goodfetch_counted_work::CountedWork::default(),
        },
    )
}

/// Covers: story:6-11
#[test]
fn each_built_in_is_chosen_by_its_name_and_dom_smoothie_by_default() {
    assert_eq!(BuiltIn::named("dom_smoothie"), Some(BuiltIn::DomSmoothie));
    assert_eq!(BuiltIn::named("trafilatura"), Some(BuiltIn::Trafilatura));
    assert_eq!(BuiltIn::named("readability"), None);
    assert_eq!(BuiltIn::default(), BuiltIn::DomSmoothie);
}

/// Covers: story:6-11
#[test]
fn the_two_built_ins_find_another_main_content_on_one_article() {
    let html = include_str!("../fixtures/navigation-and-related.html");

    let dom_smoothie = read_with(html, BuiltIn::DomSmoothie);
    let trafilatura = read_with(html, BuiltIn::Trafilatura);

    let image = "![](https://kites.test/images/line.jpg)";
    let paragraph = "Braided polyester is the usual choice for single-line kites";
    for read in [&dom_smoothie, &trafilatura] {
        assert_eq!(read.main_content, Some(MainContent::Found));
        assert!(read.markdown.as_deref().unwrap().contains(paragraph));
        assert!(read.text.as_deref().unwrap().contains(paragraph));
    }
    assert!(
        dom_smoothie.markdown.as_deref().unwrap().contains(image),
        "{dom_smoothie:?}"
    );
    assert!(
        !trafilatura.markdown.as_deref().unwrap().contains(image),
        "{trafilatura:?}"
    );
}

/// A step that finds no text where the document holds some, as
/// rs-trafilatura on a list of links alone, answers what the document
/// holds outside its navigation landmarks, never `none` (pitfall D02).
///
/// Covers: story:6-11
#[rstest]
fn a_document_with_text_outside_its_landmarks_has_main_content_whatever_the_step(
    #[values(BuiltIn::DomSmoothie, BuiltIn::Trafilatura)] built_in: BuiltIn,
) {
    let html = r#"<!doctype html><html lang="en"><head><title>Links</title></head><body>
<ul><li><a href="https://example.test/one">One</a></li><li><a href="https://example.test/two">Two</a></li></ul>
</body></html>"#;

    let read = read_with(html, built_in);

    assert_eq!(read.main_content, Some(MainContent::Found));
    let markdown = read.markdown.unwrap();
    assert!(
        markdown.contains("[One](https://example.test/one)")
            && markdown.contains("[Two](https://example.test/two)"),
        "{markdown:?}"
    );
}

/// Covers: story:6-11, story:6-12
#[rstest]
fn a_document_with_no_main_content_answers_none_whatever_the_step(
    #[values(BuiltIn::DomSmoothie, BuiltIn::Trafilatura)] built_in: BuiltIn,
) {
    let read = read_with(include_str!("../fixtures/no-main-content.html"), built_in);

    assert_eq!(read.main_content, Some(MainContent::None));
    assert_eq!(read.markdown.as_deref(), Some(""));
    assert_eq!(read.text.as_deref(), Some(""));
}
