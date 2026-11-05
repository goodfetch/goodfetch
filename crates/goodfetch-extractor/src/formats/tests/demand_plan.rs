//! The demand plan through the crate's interface: a table of requests and
//! of `frontier.learning`, each read with a clean-up that counts its runs,
//! holds that no reader outside the plan runs (spec #1577's Testing
//! Decisions).

use std::cell::Cell;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::page::FormatsRequest;
use rstest::rstest;
use serde_json::json;

use super::URL;
use crate::{CleanUp, Context, Demand, Document, Plan, Reader, Serialization, measured_formats};

/// A page with a main content.
const PAGE: &str = include_str!("../fixtures/article.html");

/// A Clean-up that removes nothing and counts each time it runs.
#[derive(Default)]
struct Counted(Cell<u32>);

impl CleanUp for Counted {
    fn remove(&self, _document: &goodfetch_counted_work::ParsedHtml, _url: &str) -> bool {
        self.0.set(self.0.get() + 1);
        false
    }
}

/// What reading `request` ran: the clean-up's runs and the main-content
/// step's.
fn ran(request: &FormatsRequest) -> (u32, u64) {
    let clean_up = Counted::default();
    let work = CountedWork::default();
    let context = Context {
        clean_up: Some(&clean_up),
        work: work.clone(),
        ..Context::default()
    };
    measured_formats(
        &Document {
            html: PAGE,
            url: URL,
            base_url: URL,
            serialization: Serialization::Served,
        },
        request,
        &context,
    );
    (clean_up.0.get(), work.read(Row::PageReadabilityRuns))
}

#[rstest]
#[case::nothing(json!({}), &[])]
#[case::raw_html(json!({ "html": { "kind": "raw" } }), &[])]
#[case::links(json!({ "links": true }), &[Reader::Untouched])]
#[case::metadata(
    json!({ "metadata": true }),
    &[Reader::Untouched, Reader::CleanUp, Reader::CleanedTree],
)]
#[case::cleaned_html(json!({ "html": { "kind": "cleaned" } }), &[Reader::CleanUp])]
#[case::full_markdown(json!({ "markdown": { "content": "full" } }), &[Reader::CleanUp])]
#[case::images(json!({ "images": true }), &[Reader::CleanUp, Reader::CleanedAssets])]
#[case::media(json!({ "media": true }), &[Reader::CleanUp, Reader::CleanedAssets])]
#[case::tables(json!({ "tables": true }), &[Reader::CleanUp, Reader::CleanedTree])]
#[case::contacts(json!({ "contacts": true }), &[Reader::CleanUp, Reader::CleanedTree])]
#[case::markdown(json!({ "markdown": true }), &[Reader::CleanUp, Reader::MainContent])]
#[case::fit(
    json!({ "markdown": { "content": "fit" } }),
    &[Reader::CleanUp, Reader::MainContent],
)]
#[case::main_html(json!({ "html": true }), &[Reader::CleanUp, Reader::MainContent])]
#[case::text(json!({ "text": true }), &[Reader::CleanUp, Reader::MainContent])]
#[case::links_and_text(
    json!({ "links": true, "text": true }),
    &[Reader::Untouched, Reader::CleanUp, Reader::MainContent],
)]
fn no_reader_outside_the_plan_runs(
    #[case] request: serde_json::Value,
    #[case] formats_readers: &[Reader],
    #[values(true, false)] learning: bool,
) {
    let request: FormatsRequest = serde_json::from_value(request).unwrap();
    let plan = Plan::of(&Demand {
        formats: &request,
        learning,
    });
    let mut expected = formats_readers.to_vec();
    if learning {
        expected.push(Reader::PageSignature);
    }
    expected.sort_unstable();
    let mut planned: Vec<Reader> = plan.readers().collect();
    planned.sort_unstable();
    assert_eq!(planned, expected);
    let (clean_ups, main_contents) = ran(&request);
    assert_eq!(
        (clean_ups, main_contents),
        (
            u32::from(plan.runs(Reader::CleanUp)),
            u64::from(plan.runs(Reader::MainContent)),
        ),
        "the clean-up's and the main-content step's runs against the plan {plan:?}",
    );
}
