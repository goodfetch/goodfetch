//! `page_signature` through the crate's interface: two documents match
//! when their titles and the text of their main content match, white space
//! aside, whatever else differs; no owned code is mocked (ADR 0062).

use rstest::rstest;

use crate::{BuiltIn, Document, Serialization, page_signature};

const URL: &str = "https://kites.test/events/2026/10";

fn page(title: &str, main: &str, nav: &str) -> String {
    format!(
        "<!doctype html><html><head><title>{title}</title></head><body>\
         <nav>{nav}</nav><main><article><h1>Events</h1><p>{main}</p>\
         <p>Every kite meeting of the club is listed here, with its field and its time, \
         so that members can plan a weekend of flying together.</p></article></main>\
         <footer>Kite club</footer></body></html>"
    )
}

fn signature(html: &str, url: &str) -> [u8; 32] {
    page_signature(
        &Document {
            html,
            url,
            base_url: url,
            serialization: Serialization::Served,
        },
        BuiltIn::default().extractor(),
        &goodfetch_counted_work::CountedWork::default(),
    )
}

/// The same title and main text match, though the navigation, the links,
/// the white space and the URL differ.
#[rstest]
#[case::navigation(
    page("Events", "No events this month.", "<a href=\"/2026/09\">Previous</a>"),
    page(
        "Events",
        "No events this month.",
        "<a href=\"/2026/11\">Next</a> <a href=\"/\">Home</a>"
    )
)]
#[case::white_space(
    page("Events", "No events this month.", ""),
    page("  Events\n", "No   events\n\tthis month.", "")
)]
#[case::links_in_the_text(
    page("Events", "No events this <a href=\"/a?ref=1\">month</a>.", ""),
    page("Events", "No events this <a href=\"/a?ref=2\">month</a>.", "")
)]
fn pages_that_read_the_same_match(#[case] one: String, #[case] other: String) {
    assert_eq!(
        signature(&one, URL),
        signature(&other, "https://kites.test/events/2026/11")
    );
}

/// A title or a main text that differs makes another signature, and so
/// does text moved from the title into the main content.
#[rstest]
#[case::title(
    page("Events", "No events this month.", ""),
    page("Events, October", "No events this month.", "")
)]
#[case::main_text(
    page("Events", "No events this month.", ""),
    page("Events", "One event this month.", "")
)]
#[case::text_moved(
    page("Events A", "No events this month.", ""),
    page("Events", "A No events this month.", "")
)]
fn pages_that_read_differently_differ(#[case] one: String, #[case] other: String) {
    assert_ne!(signature(&one, URL), signature(&other, URL));
}

/// A document with no main content is signed by its title alone, so two
/// navigation-only pages of one title match.
#[test]
fn a_page_with_no_main_content_is_signed_by_its_title() {
    let navigation = |links: &str| {
        format!("<html><head><title>Menu</title></head><body><nav>{links}</nav></body></html>")
    };
    let one = signature(&navigation("<a href=\"/a\">A</a>"), URL);
    let other = signature(&navigation("<a href=\"/b\">B</a>"), URL);
    let titled = signature(
        "<html><head><title>Other</title></head><body><nav></nav></body></html>",
        URL,
    );
    assert_eq!(one, other);
    assert_ne!(one, titled);
}
