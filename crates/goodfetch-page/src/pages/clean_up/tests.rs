//! A page's formats read with the Clean-up rules: what the main-content
//! step costs a call whose rules removed an element.

use std::sync::mpsc::{self, Sender};

use goodfetch_extractor::{BuiltIn, Extractor, Serialization};
use goodfetch_protocol::page::{FormatsRequest, PageId};
use serde_json::{Value, json};

use super::read;
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
    fn main_content(&self, html: &str, url: &str) -> Option<String> {
        self.runs.send(()).unwrap();
        BuiltIn::default().extractor().main_content(html, url)
    }
}

/// The main-content step's runs and the Provenance entry of one call
/// asking `request` of [`ARTICLE`], its ad hidden.
fn read_with_the_ad_hidden(request: Value) -> (usize, Value) {
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
        &document,
        Serialization::Served,
        request,
        &counted,
        (None, &hidden),
    );

    (
        reported.try_iter().count(),
        done.entry(&PageId("page-1".to_owned()), None),
    )
}

/// The cleaned document's main content is read once, by the formats, and
/// the document as served once more, for the share the rules took.
/// Covers: story:6-10
#[test]
fn a_call_whose_rules_removed_an_element_reads_each_documents_main_content_once() {
    let (runs, entry) = read_with_the_ad_hidden(json!({ "markdown": true }));

    assert_eq!(runs, 2);
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
    let (runs, entry) = read_with_the_ad_hidden(json!({ "links": true }));

    assert_eq!(
        (
            runs,
            &entry["main_content"],
            entry["removed"].as_array().map(Vec::len)
        ),
        (0, &Value::Null, Some(1))
    );
}
