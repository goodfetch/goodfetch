//! A page's formats read with the Clean-up rules: what the main-content
//! step costs a call whose rules removed an element.

use std::sync::mpsc::{self, Sender};

use goodfetch_counted_work::ParsedHtml;
use goodfetch_extractor::{BuiltIn, Extractor, Found, Serialization};
use goodfetch_protocol::page::{FormatsRequest, PageId};
use serde_json::{Value, json};

use super::{ShareMeasure, read};
use crate::driver::SerializedDocument;

/// An article with an ad the Consent prompt runner's selector hides.
const ARTICLE: &str = "<!doctype html><html><head><title>Lines</title></head><body><article>\
    <h1>Choosing a kite line</h1>\
    <p>A kite line has to be strong enough for the pull of the kite in the strongest wind you fly in, and light enough that it does not sag between your hand and the kite on a calm day.</p>\
    <div class=\"ad-slot\"><p>Buy a reel today, at half its price, for one week only.</p></div>\
    <p>Braided polyester is the usual choice for single-line kites: it stretches a little, which softens gusts, and it is cheap enough to replace when it frays on a rough reel.</p>\
    </article></body></html>";

/// A main-content step that reports each of its runs.
struct Counted {
    runs: Sender<()>,
}

impl Extractor for Counted {
    fn main_content(&self, document: ParsedHtml, url: &str) -> Option<Found> {
        self.runs.send(()).unwrap();
        BuiltIn::default().extractor().main_content(document, url)
    }
}

/// The main-content step's runs and the Provenance entry of one call
/// asking `request` of [`ARTICLE`], its ad hidden, its share measured as
/// `measure` says; none when the call ran no rule.
fn read_measured(request: Value, measure: ShareMeasure) -> (usize, Option<Value>) {
    let (runs, reported) = mpsc::channel();
    let counted = Counted { runs };
    let document = SerializedDocument {
        html: ARTICLE.to_owned(),
        url: "https://kites.test/journal/lines".to_owned(),
        base_url: "https://kites.test/journal/lines".to_owned(),
    };
    let request: FormatsRequest = serde_json::from_value(request).unwrap();
    let hidden = [".ad-slot".to_owned()];

    let (_, done) = read(
        (&document, None),
        Serialization::Served,
        request,
        (&counted, measure),
        (None, &hidden),
        &goodfetch_counted_work::CountedWork::default(),
    );

    (
        reported.try_iter().count(),
        done.map(|done| done.entry(&PageId("page-1".to_owned()), None)),
    )
}

/// [`read_measured`] with the default measure, the estimate.
fn read_with_the_ad_hidden(request: Value) -> (usize, Option<Value>) {
    read_measured(request, ShareMeasure::default())
}

/// By default the cleaned document's main content is read once, by the
/// formats, and the share the rules took is estimated from it: the text
/// they removed counts as what the main content lost, so the main content
/// before is the text left and the text removed (ADR 0138).
/// Covers: story:6-10
#[test]
fn by_default_a_call_whose_rules_removed_an_element_reads_its_main_content_once_and_estimates_the_share()
 {
    let (runs, entry) = read_with_the_ad_hidden(json!({ "markdown": true }));
    let entry = entry.unwrap();

    let main_content = &entry["main_content"];
    assert_eq!(
        (
            runs,
            &entry["share_measure"],
            main_content["before"].as_u64()
        ),
        (
            1,
            &json!("estimate"),
            main_content["after"]
                .as_u64()
                .zip(entry["removed_text"].as_u64())
                .map(|(after, removed)| after + removed)
        ),
        "{entry}"
    );
}

/// Under `extraction.removal_share: exact` the cleaned document's main
/// content is read once, by the formats, and the document as served once
/// more, for the share the rules took.
/// Covers: story:6-10
#[test]
fn under_an_exact_share_a_call_whose_rules_removed_an_element_reads_each_documents_main_content_once()
 {
    let (runs, entry) = read_measured(json!({ "markdown": true }), ShareMeasure::Exact);
    let entry = entry.unwrap();

    assert_eq!((runs, &entry["share_measure"]), (2, &json!("exact")));
    assert!(
        entry["main_content"]["before"].as_u64() > entry["main_content"]["after"].as_u64(),
        "{entry}"
    );
}

/// A call that reads nothing from the main content measures no share, so
/// its rules' removal costs it no main-content step.
/// Covers: story:6-10
#[test]
fn a_call_with_no_format_of_the_main_content_reads_no_main_content_for_its_share() {
    let (runs, entry) = read_with_the_ad_hidden(json!({ "html": { "kind": "cleaned" } }));
    let entry = entry.unwrap();

    assert_eq!(
        (
            runs,
            &entry["main_content"],
            entry["removed"].as_array().map(Vec::len)
        ),
        (0, &Value::Null, Some(1))
    );
}

/// A call that reads nothing of the cleaned page, its links alone, runs
/// no rule and leaves Provenance no line of it (ADR 0138's demand plan).
/// Covers: story:6-10
#[test]
fn a_call_reading_nothing_of_the_cleaned_page_runs_no_rule_and_records_none() {
    let (runs, entry) = read_with_the_ad_hidden(json!({ "links": true }));

    assert_eq!((runs, entry), (0, None));
}

/// A browser page's formats read takes one serialization of its DOM and
/// parses it once, whatever it asks for: every format, the runner's hidden
/// selector removed in place on that one tree, with no serialization handed
/// between modules but the cleaned HTML it asks for (ADR 0138).
#[test]
fn a_browser_pages_formats_read_parses_its_serialization_once_with_every_format() {
    let work = goodfetch_counted_work::CountedWork::default();
    let document = SerializedDocument {
        html: ARTICLE.replace(
            "</article>",
            "<table><tr><th>Line</th></tr><tr><td>Polyester</td></tr></table>\
             <a href=\"mailto:kites@kites.test\">Write</a><img src=\"/reel.png\" alt=\"A reel\"></article>",
        ),
        url: "https://kites.test/journal/lines".to_owned(),
        base_url: "https://kites.test/journal/lines".to_owned(),
    };
    let request: FormatsRequest = serde_json::from_value(json!({
        "markdown": true,
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
    let hidden = [".ad-slot".to_owned()];

    let (formats, done) = read(
        &document,
        Serialization::Dom,
        request,
        (BuiltIn::default().extractor(), ShareMeasure::default()),
        (None, &hidden),
        &work,
    );

    assert_eq!(work.read(goodfetch_counted_work::Row::PageParses), 1);
    assert!(!formats.html.unwrap().contains("ad-slot"));
    assert!(done.is_some_and(|done| !done.removed.is_empty()));
}
