//! The weighing over pairs of staged runs (spec #579's Testing Decisions).
//!
//! Covers: story:5-114, story:5-116

use std::collections::BTreeSet;

use goodfetch_protocol::foxhound::crawl::PushedRecord;
use rstest::rstest;
use serde_json::{Value, json};

use super::{StagedRun, weigh};

const PAGE: &str = "http://tiers.test/products/1";

fn records(data: &[Value]) -> Vec<PushedRecord> {
    data.iter()
        .map(|data| PushedRecord {
            dataset: "pages".to_owned(),
            data: data.clone(),
        })
        .collect()
}

fn identities(identities: &[&str]) -> BTreeSet<String> {
    identities
        .iter()
        .map(|identity| (*identity).to_owned())
        .collect()
}

/// Whether a cheaper run that staged `cheaper` and a browser run that
/// staged `browser`, each with no enqueue, on one URL in one encoding,
/// match.
fn records_match(cheaper: &[Value], browser: &[Value], removed: &[&str]) -> bool {
    let (cheaper, browser) = (records(cheaper), records(browser));
    let none = BTreeSet::new();
    weigh(
        &StagedRun {
            records: &cheaper,
            enqueues: &none,
            ended_at: Some(PAGE),
            encoding: Some("UTF-8"),
        },
        &StagedRun {
            records: &browser,
            enqueues: &none,
            ended_at: Some(PAGE),
            encoding: Some("UTF-8"),
        },
        &identities(removed),
    )
}

/// Records are compared as multisets of canonical JSON after NFC and
/// whitespace normalization of every string, the fields the removal took
/// out of the cheaper run's content left out.
#[rstest]
#[case::the_same_records_in_another_order(
    &[json!({ "n": 1 }), json!({ "n": 2 })], &[json!({ "n": 2 }), json!({ "n": 1 })], &[], true
)]
#[case::strings_that_differ_only_in_whitespace(
    &[json!({ "title": "  Bench\n\t chair " })], &[json!({ "title": "Bench chair" })], &[], true
)]
#[case::strings_that_differ_only_in_unicode_composition(
    &[json!({ "title": "Cafe\u{301}" })], &[json!({ "title": "Caf\u{e9}" })], &[], true
)]
#[case::keys_in_another_order_nested(
    &[json!({ "a": { "x": 1, "y": [ " b " ] }, "b": 2 })],
    &[json!({ "b": 2, "a": { "y": [ "b" ], "x": 1 } })],
    &[],
    true
)]
#[case::strings_that_differ_in_their_words(
    &[json!({ "title": "Bench" })], &[json!({ "title": "Bench chair" })], &[], false
)]
#[case::a_record_twice_against_two_records(
    &[json!({ "n": 1 }), json!({ "n": 1 })], &[json!({ "n": 1 }), json!({ "n": 2 })], &[], false
)]
#[case::one_record_fewer(&[json!({ "n": 1 })], &[json!({ "n": 1 }), json!({ "n": 1 })], &[], false)]
#[case::a_number_against_its_text(&[json!({ "n": 1 })], &[json!({ "n": "1" })], &[], false)]
#[case::a_field_the_removal_took_out_of_the_cheaper_run(
    &[json!({ "title": "Bench", "teaser": "Ad: buy now" })],
    &[json!({ "title": "Bench" })],
    &["teaser"],
    true
)]
#[case::a_field_no_removal_names(
    &[json!({ "title": "Bench", "teaser": "Ad: buy now" })], &[json!({ "title": "Bench" })], &[], false
)]
#[case::nothing_staged_on_either(&[], &[], &[], true)]
fn records_match_as_multisets_of_normalized_canonical_json(
    #[case] cheaper: &[Value],
    #[case] browser: &[Value],
    #[case] removed: &[&str],
    #[case] matched: bool,
) {
    assert_eq!(records_match(cheaper, browser, removed), matched);
}

/// Enqueues are compared as sets of Frontier identities: a listing whose
/// cheaper run found none of its links never matches the browser's.
#[rstest]
#[case::the_same_requests(&["GET http://tiers.test/a", "GET http://tiers.test/b"], &["GET http://tiers.test/b", "GET http://tiers.test/a"], true)]
#[case::no_request_where_the_browser_staged_some(&[], &["GET http://tiers.test/a"], false)]
#[case::another_body_hash(&["POST http://tiers.test/a 00"], &["POST http://tiers.test/a 01"], false)]
#[case::another_method(&["GET http://tiers.test/a"], &["HEAD http://tiers.test/a"], false)]
fn enqueues_match_as_sets_of_frontier_identities(
    #[case] cheaper: &[&str],
    #[case] browser: &[&str],
    #[case] matched: bool,
) {
    let (cheaper, browser) = (identities(cheaper), identities(browser));
    let staged = |enqueues| StagedRun {
        records: &[],
        enqueues,
        ended_at: Some(PAGE),
        encoding: None,
    };
    assert_eq!(
        weigh(&staged(&cheaper), &staged(&browser), &BTreeSet::new()),
        matched
    );
}

/// A browser run that ended on another URL (a script redirect, ADR 0118)
/// or read its document in another `document.characterSet` (ADR 0119)
/// never matches, whatever the Records; URLs compare by their Frontier
/// key, encodings without regard to case, and a run that names neither
/// leaves the Records to decide.
///
/// Covers: story:5-117
#[rstest]
#[case::one_url_one_encoding(Some(PAGE), Some("UTF-8"), Some(PAGE), Some("utf-8"), true)]
#[case::another_url(Some(PAGE), None, Some("http://tiers.test/products/2"), None, false)]
#[case::the_same_url_keyed(Some("http://TIERS.test:80/products/1"), None, Some(PAGE), None, true)]
#[case::another_encoding(Some(PAGE), Some("UTF-8"), Some(PAGE), Some("windows-1252"), false)]
#[case::a_browser_that_names_neither(Some(PAGE), Some("EUC-KR"), None, None, true)]
fn a_browser_run_on_another_url_or_in_another_encoding_never_matches(
    #[case] cheaper_url: Option<&str>,
    #[case] cheaper_encoding: Option<&str>,
    #[case] browser_url: Option<&str>,
    #[case] browser_encoding: Option<&str>,
    #[case] matched: bool,
) {
    let same = records(&[json!({ "n": 1 })]);
    let none = BTreeSet::new();
    let staged = |ended_at, encoding| StagedRun {
        records: &same,
        enqueues: &none,
        ended_at,
        encoding,
    };
    assert_eq!(
        weigh(
            &staged(cheaper_url, cheaper_encoding),
            &staged(browser_url, browser_encoding),
            &BTreeSet::new()
        ),
        matched
    );
}
