//! The main-content step `content.extractor` chooses (#788): each
//! built-in by its name, the main content each finds where they differ,
//! and the rules the composition holds every step to, whichever it is.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::page::{FormatsRequest, MainContent, PageFormats};
use rstest::rstest;
use serde_json::json;

use super::URL;
use crate::{Attempts, BuiltIn, Chosen, Context, Document, Serialization, formats};

/// `built_in` with the default attempts.
fn chosen(built_in: BuiltIn) -> Chosen {
    Chosen {
        built_in,
        ..Chosen::default()
    }
}

/// The Markdown and the text the `chosen` step reads from `html` at
/// [`URL`].
fn read_with(html: &str, chosen: Chosen) -> PageFormats {
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
            extractor: chosen.extractor(),
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

    let dom_smoothie = read_with(html, chosen(BuiltIn::DomSmoothie));
    let trafilatura = read_with(html, chosen(BuiltIn::Trafilatura));

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

    let read = read_with(html, chosen(built_in));

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
    let read = read_with(
        include_str!("../fixtures/no-main-content.html"),
        chosen(built_in),
    );

    assert_eq!(read.main_content, Some(MainContent::None));
    assert_eq!(read.markdown.as_deref(), Some(""));
    assert_eq!(read.text.as_deref(), Some(""));
}

/// rs-trafilatura reads the tree it is given, never a parse of that tree's
/// serialization (#1916): a script whose text, set on the tree, closes the
/// script and opens a paragraph is a script on the tree, and a paragraph
/// only once the serialization is parsed again.
///
/// Covers: story:6-11
#[test]
fn trafilatura_reads_the_tree_it_is_given_never_a_parse_of_its_serialization() {
    let html = r#"<!doctype html><html lang="en"><head><title>Bridles</title></head><body>
<article><h1>Bridles</h1>
<p>A two-leg bridle sets the angle a single-line kite meets the wind at, and moving its towing point a centimetre changes how the kite climbs.</p>
<script></script>
<p>Tie the towing point with a lark's head knot, so it slides when you want it to and holds when the line is under load.</p>
</article></body></html>"#;
    let work = CountedWork::default();
    let tree = work.parse_html(Row::PageParses, html);
    let script = work
        .compile_selector(Row::PageSelectorCompilations, "script")
        .unwrap();
    tree.select_matcher(&script).set_text(
        "</script><p>A paragraph that only a parse of the tree's serialization holds, long enough to read as content.</p><script>",
    );

    let found = chosen(BuiltIn::Trafilatura)
        .extractor()
        .main_content(tree, URL)
        .unwrap();

    assert!(found.text.contains("lark's head knot"), "{found:?}");
    assert!(!found.text.contains("only a parse"), "{found:?}");
}

/// Covers: setting:extraction.readability
#[test]
fn each_number_of_attempts_is_chosen_by_its_name_and_retry_by_default() {
    assert_eq!(Attempts::named("retry"), Some(Attempts::Retry));
    assert_eq!(
        Attempts::named("single_attempt"),
        Some(Attempts::SingleAttempt)
    );
    assert_eq!(Attempts::named("once"), None);
    assert_eq!(
        Chosen::default(),
        Chosen {
            built_in: BuiltIn::DomSmoothie,
            attempts: Attempts::Retry,
        }
    );
}

/// A short page whose text sits in a block a first attempt drops as
/// unlikely content: one longer than its other text.
const SHORT_PAGE: &str = r#"<!doctype html><html lang="en"><head><title>Knots</title></head><body>
<div class="comment"><p>A lark's head knot holds a bridle to the towing ring, and a figure-eight loop at the end of the flying line keeps it from slipping under load.</p>
<p>Tie both before the first launch of the day, and check them again after a hard landing, since a knot that has worked loose shows it only when the line pulls.</p></div>
<div><p>Questions go to the club.</p></div>
</body></html>"#;

/// `retry` tries `dom_smoothie` again while it finds under 500 characters
/// and keeps the best it found; `single_attempt` keeps what its one
/// attempt, the first `retry` makes, finds.
///
/// Covers: setting:extraction.readability
#[test]
fn a_retry_keeps_text_a_single_attempt_drops_on_a_short_page() {
    let retry = read_with(SHORT_PAGE, chosen(BuiltIn::DomSmoothie));
    let single = read_with(
        SHORT_PAGE,
        Chosen {
            built_in: BuiltIn::DomSmoothie,
            attempts: Attempts::SingleAttempt,
        },
    );

    let knot = "lark's head knot";
    assert!(retry.text.as_deref().unwrap().contains(knot), "{retry:?}");
    assert!(
        !single.text.as_deref().unwrap().contains(knot),
        "{single:?}"
    );
}

/// rs-trafilatura tries once whatever `extraction.readability` says.
///
/// Covers: setting:extraction.readability
#[test]
fn trafilatura_reads_the_same_whatever_the_attempts() {
    let html = include_str!("../fixtures/navigation-and-related.html");
    let read = |attempts| {
        read_with(
            html,
            Chosen {
                built_in: BuiltIn::Trafilatura,
                attempts,
            },
        )
    };

    assert_eq!(read(Attempts::SingleAttempt), read(Attempts::Retry));
}

/// The Page signature reads the main text the formats read when both run
/// the same step on a page no Clean-up rule changes, under each value of
/// `extraction.readability`, white space aside.
///
/// Covers: setting:extraction.readability
#[rstest]
fn the_page_signature_reads_the_main_text_the_formats_read_with_one_step(
    #[values(BuiltIn::DomSmoothie, BuiltIn::Trafilatura)] built_in: BuiltIn,
    #[values(Attempts::Retry, Attempts::SingleAttempt)] attempts: Attempts,
    #[values(
        SHORT_PAGE,
        include_str!("../fixtures/navigation-and-related.html"),
        include_str!("../fixtures/article.html")
    )]
    html: &str,
) {
    let chosen = Chosen { built_in, attempts };
    let work = CountedWork::default();

    let (_, signed) = crate::main_content::title_and_main_text_of(
        work.parse_html(Row::PageParses, html),
        URL,
        chosen.extractor(),
        &work,
    );
    let formats = read_with(html, chosen);

    // The formats' text puts a break between blocks the signature's runs
    // together.
    let unspaced = |text: &str| text.split_whitespace().collect::<String>();
    assert_eq!(
        unspaced(&signed),
        unspaced(formats.text.as_deref().unwrap())
    );
}
