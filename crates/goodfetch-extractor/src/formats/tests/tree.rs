//! The main-content step on the page's one tree (ADR 0138), through the
//! crate's interface: what a read of the main content and a Page signature
//! count, and the fallback a step that finds nothing answers.

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_protocol::page::FormatsRequest;
use rstest::rstest;
use serde_json::json;

use super::URL;
use crate::{
    BuiltIn, CleanUp, Context, Document, Extractor, Found, Serialization, measured_formats,
    page_signature,
};

/// A page with a main content.
const PAGE: &str = include_str!("../fixtures/article.html");

/// A Clean-up that removes nothing.
struct Keeps;

impl CleanUp for Keeps {
    fn remove(&self, _document: &ParsedHtml, _url: &str) -> bool {
        false
    }
}

/// A step that finds no main content in any page.
struct FindsNothing;

impl Extractor for FindsNothing {
    fn main_content(&self, _document: ParsedHtml, _url: &str) -> Option<Found> {
        None
    }
}

fn document(html: &str) -> Document<'_> {
    Document {
        html,
        url: URL,
        base_url: URL,
        serialization: Serialization::Served,
    }
}

/// The page's parses, tree copies and runs of the main-content step on
/// `work`.
fn counted(work: &CountedWork) -> (u64, u64, u64) {
    (
        work.read(Row::PageParses),
        work.read(Row::PageTreeCopies),
        work.read(Row::PageReadabilityRuns),
    )
}

/// A read of the main content after the Clean-up rules parses the page
/// once, for the clean-up, whose tree the step then reads, and copies it
/// once, for the fallback that removes the landmarks: one run of the step
/// (`F1.readability_runs` ≤ 1, `F1.tree_copies` ≤ 1).
///
/// rs-trafilatura reads that tree as dom-smoothie does (#1916).
#[rstest]
#[case::markdown(json!({ "markdown": true }))]
#[case::text(json!({ "text": true }))]
#[case::main_html(json!({ "html": true }))]
fn a_read_of_the_main_content_parses_the_page_once_and_copies_its_tree_once(
    #[case] request: serde_json::Value,
    #[values(BuiltIn::DomSmoothie, BuiltIn::Trafilatura)] built_in: BuiltIn,
) {
    let request: FormatsRequest = serde_json::from_value(request).unwrap();
    let work = CountedWork::default();
    let context = Context {
        clean_up: Some(&Keeps),
        extractor: built_in.extractor(),
        work: work.clone(),
    };

    let measured = measured_formats(&document(PAGE), &request, &context);

    assert!(
        measured
            .main_content_length
            .is_some_and(|length| length > 0)
    );
    assert_eq!(counted(&work), (1, 1, 1));
}

/// A shadow root, inside the article.
const SHADOW_PAGE: &str = "<html><head><title>Kites</title></head><body><nav><a href=\"/\">Home</a></nav>\
     <article><h1>Bridles</h1><div><template shadowrootmode=\"open\"><p>A bridle sets the angle a \
     kite meets the wind at, and each of its legs is tied to the spar at a point the maker \
     measured.</p><slot></slot></template><p>Shorten the top leg to fly in stronger wind.</p>\
     </div><table><tr><th>Wind</th><td>Leg</td></tr></table><p>Write to \
     <a href=\"mailto:club@kites.test\">the club</a>.</p><ul><li><a href=\"/a\">A</a></li></ul>\
     <img src=\"/kite.png\" alt=\"A kite\"></article><div class=\"ad\">Buy a reel</div>\
     <script type=\"application/ld+json\">{\"@type\": \"Article\"}</script></body></html>";

/// A Clean-up that removes every `div.ad`.
struct RemovesAds;

impl CleanUp for RemovesAds {
    fn remove(&self, document: &ParsedHtml, _url: &str) -> bool {
        let ads = document.select_matcher(
            &CountedWork::default().literal_selector(Row::PageSelectorCompilations, "div.ad"),
        );
        let removed = ads.exists();
        ads.remove();
        removed
    }
}

/// Every format asked of a page the Clean-up rules change, its shadow
/// roots composed, reads one tree of it (`F1.parses` ≤ 1 with every
/// format on, ADR 0138): the clean-up's, composed in place and read by
/// every reader of the cleaned page before the main-content step consumes
/// it, with the one copy of it the landmarks fallback needs.
#[rstest]
#[case::rules_keep_the_page(&Keeps as &dyn CleanUp)]
#[case::rules_remove_an_ad(&RemovesAds as &dyn CleanUp)]
fn every_format_asked_reads_one_tree_of_the_page(#[case] clean_up: &dyn CleanUp) {
    let request: FormatsRequest = serde_json::from_value(json!({
        "markdown": { "content": "fit" },
        "html": { "kind": "cleaned" },
        "text": true,
        "links": true,
        "images": true,
        "media": true,
        "tables": true,
        "metadata": true,
        "contacts": true,
    }))
    .unwrap();
    let work = CountedWork::default();
    let context = Context {
        clean_up: Some(clean_up),
        work: work.clone(),
        ..Context::default()
    };

    let measured = measured_formats(&document(SHADOW_PAGE), &request, &context);

    let formats = measured.formats;
    assert!(formats.text.unwrap().contains("A bridle sets the angle"));
    assert_eq!(formats.tables.unwrap().len(), 1);
    assert_eq!(formats.contacts.unwrap().emails.len(), 1);
    assert_eq!(formats.metadata.unwrap().json_ld.len(), 1);
    assert_eq!(formats.images.unwrap().len(), 1);
    assert_eq!(counted(&work), (1, 1, 1));
}

/// A Page signature reads the untouched tree it is given with one run of
/// the step, and no copy: the fallback's text is read before the step
/// consumes the tree.
#[test]
fn a_page_signature_runs_the_step_once_on_one_parse_and_copies_nothing() {
    let work = CountedWork::default();

    let _ = page_signature(
        &document(PAGE),
        crate::BuiltIn::default().extractor(),
        &work,
    );

    assert_eq!(counted(&work), (1, 0, 1));
}

/// A step that finds no main content in a page holding text outside its
/// landmarks leaves the formats and the Page signature that text, as the
/// formats always read it (pitfall D02): two pages of one title whose text
/// differs sign differently.
#[test]
fn a_page_the_step_finds_nothing_in_is_signed_by_its_text_outside_its_landmarks() {
    let page = |text: &str| {
        format!(
            "<html><head><title>Kites</title></head><body><nav><a href=\"/\">Home</a></nav>\
             <p>{text}</p><footer>Kite club</footer></body></html>"
        )
    };
    let (one, other) = (page("A red kite."), page("A blue kite."));
    let work = CountedWork::default();

    let signed = |html: &str| page_signature(&document(html), &FindsNothing, &work);

    assert_ne!(signed(&one), signed(&other));
    assert_eq!(signed(&one), signed(&page("A   red\nkite.")));
}
