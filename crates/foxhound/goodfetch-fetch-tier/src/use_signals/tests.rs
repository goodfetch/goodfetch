//! The AI-use signals' judgment as a function of a response's headers,
//! its document's markup, the files a site declares them in, and the
//! Purpose (spec #581's pure rules, tested in process).
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use goodfetch_counted_work::{CountedWork, Row};
use rstest::rstest;
use url::Url;

use super::{Purpose, UseSignals, content_signals_refusing, rsl_refusing, tdmrep_file_refusing};
use crate::untouched::{UntouchedDemand, UntouchedReaders};

/// The signals `headers` and the markup `html` of a document a handler's
/// page navigated to at `at` carry, as the navigation gate reads them, and
/// the parses that read took.
fn read_markup(headers: &[(String, String)], html: &str, at: &Url) -> (UseSignals, u64) {
    let work = CountedWork::default();
    let readers = UntouchedReaders::of(UntouchedDemand::default(), &work);
    let before = work.read(Row::PageParses);
    let signals = readers.markup_use_signals(UseSignals::of(headers), html, at, &work);
    (signals, work.read(Row::PageParses) - before)
}

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// Which of `ai-input`, `ai-train` and `collect` the headers refuse.
fn refused(pairs: &[(&str, &str)]) -> [bool; 3] {
    let signals = UseSignals::of(&headers(pairs));
    [Purpose::AiInput, Purpose::AiTrain, Purpose::Collect]
        .map(|purpose| signals.refusing(purpose).is_some())
}

#[rstest]
#[case::no_signal(&[], [false, false, false])]
#[case::a_tdm_reservation(&[("tdm-reservation", "1")], [true, true, true])]
#[case::a_tdm_reservation_in_any_case(&[("TDM-Reservation", " 1 ")], [true, true, true])]
#[case::no_tdm_reservation(&[("tdm-reservation", "0")], [false, false, false])]
#[case::aipref_refusing_training(&[("content-usage", "train-ai=n")], [false, true, false])]
#[case::aipref_refusing_ai_use(&[("content-usage", "ai-use=n, search=y")], [true, false, false])]
#[case::aipref_refusing_all_processing(&[("content-usage", "bulk=n")], [true, true, true])]
#[case::the_narrower_preference_wins(&[("content-usage", "bulk=n, train-ai=y")], [true, false, true])]
#[case::aipref_allowing(&[("content-usage", "train-ai=y")], [false, false, false])]
#[case::aipref_unknown_values_say_nothing(&[("content-usage", "train-ai, ai-use=maybe")], [false, false, false])]
fn a_signal_refuses_the_purposes_it_names(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: [bool; 3],
) {
    assert_eq!(refused(pairs), expected);
}

#[test]
fn no_signal_is_judged_against_search() {
    let signals = UseSignals::of(&headers(&[
        ("tdm-reservation", "1"),
        ("content-usage", "bulk=n, search=n"),
    ]));

    assert_eq!(signals.refusing(Purpose::Search), None);
}

#[rstest]
#[case("search", Some(Purpose::Search))]
#[case("ai-input", Some(Purpose::AiInput))]
#[case("ai-train", Some(Purpose::AiTrain))]
#[case("collect", Some(Purpose::Collect))]
#[case("training", None)]
fn a_purpose_is_named_as_the_setting_names_it(
    #[case] name: &str,
    #[case] expected: Option<Purpose>,
) {
    assert_eq!(Purpose::named(name), expected);
}

/// Which of `ai-input`, `ai-train` and `collect` `judge` refuses.
fn refusals(judge: impl Fn(Purpose) -> Option<&'static str>) -> [bool; 3] {
    [Purpose::AiInput, Purpose::AiTrain, Purpose::Collect].map(|purpose| judge(purpose).is_some())
}

fn signals(pairs: &[(&str, bool)]) -> Vec<(String, bool)> {
    pairs
        .iter()
        .map(|(name, allowed)| ((*name).to_owned(), *allowed))
        .collect()
}

#[rstest]
#[case::none(&[], [false, false, false])]
#[case::refusing_training(&[("search", true), ("ai-input", true), ("ai-train", false)], [false, true, false])]
#[case::refusing_input(&[("ai-input", false)], [true, false, false])]
#[case::the_last_line_wins(&[("ai-train", false), ("ai-train", true)], [false, false, false])]
#[case::search_refused_alone(&[("search", false)], [false, false, false])]
fn a_content_signal_refuses_the_purpose_it_names(
    #[case] pairs: &[(&str, bool)],
    #[case] expected: [bool; 3],
) {
    let signals = signals(pairs);

    assert_eq!(
        refusals(|purpose| content_signals_refusing(&signals, purpose)),
        expected
    );
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// The Proving ground's `access-policy` file: its catalog reserved.
const CATALOG_RESERVED: &str = r#"[{"location":"/catalog*","tdm-reservation":1,"tdm-policy":"https://example.test/tdm-policy.json"}]"#;

/// A file whose first rule leaves `/open/` free and whose second
/// reserves the rest.
const OPEN_THEN_RESERVED: &str =
    r#"[{"location":"/open/*","tdm-reservation":0},{"location":"/","tdm-reservation":1}]"#;

#[rstest]
#[case::reserved_under_its_location(CATALOG_RESERVED, "https://example.test/catalog/1", true)]
#[case::elsewhere_not(CATALOG_RESERVED, "https://example.test/about", false)]
#[case::the_first_match_wins(OPEN_THEN_RESERVED, "https://example.test/open/a", false)]
#[case::a_later_rule_when_the_first_does_not_match(
    OPEN_THEN_RESERVED,
    "https://example.test/closed",
    true
)]
#[case::an_anchored_location(
    r#"[{"location":"/*.pdf$","tdm-reservation":1}]"#,
    "https://example.test/a.pdf?x",
    false
)]
#[case::an_anchored_location_matching(
    r#"[{"location":"/*.pdf$","tdm-reservation":1}]"#,
    "https://example.test/doc/a.pdf",
    true
)]
#[case::matched_in_its_case(
    r#"[{"location":"/Catalog","tdm-reservation":1}]"#,
    "https://example.test/catalog",
    false
)]
#[case::an_invalid_value_is_unset(
    r#"[{"location":"/","tdm-reservation":2}]"#,
    "https://example.test/a",
    false
)]
#[case::no_json("not json", "https://example.test/a", false)]
fn a_tdmrep_file_reserves_what_its_first_matching_rule_does(
    #[case] file: &str,
    #[case] at: &str,
    #[case] reserved: bool,
) {
    assert_eq!(
        refusals(|purpose| tdmrep_file_refusing(file.as_bytes(), &url(at), purpose)),
        [reserved; 3]
    );
}

/// A license as RSL 1.0 writes one, holding `body`.
fn license(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><rsl xmlns="https://rslstandard.org/rsl"><content url="https://example.test/"><license>{body}</license></content></rsl>"#
    )
}

#[rstest]
#[case::prohibiting_training(
    license(r#"<prohibits type="usage">ai-train</prohibits>"#),
    [false, true, false]
)]
#[case::prohibiting_all_ai(license(r#"<prohibits type="usage">ai-all</prohibits>"#), [true, true, false])]
#[case::prohibiting_all(license(r#"<prohibits type="usage">all</prohibits>"#), [true, true, true])]
#[case::permitting_search_alone(license(r#"<permits type="usage">search</permits>"#), [true, true, true])]
#[case::permitting_input(
    license(r#"<permits type="usage">search ai-input</permits>"#),
    [false, true, true]
)]
#[case::a_prohibition_wins(
    license(r#"<permits type="usage">ai-all</permits><prohibits type="usage">ai-train</prohibits>"#),
    [false, true, true]
)]
#[case::other_types_say_nothing(
    license(r#"<permits type="user">commercial</permits>"#),
    [false, false, false]
)]
#[case::no_license(
    r#"<rsl xmlns="https://rslstandard.org/rsl"></rsl>"#.to_owned(),
    [false, false, false]
)]
#[case::one_license_permitting_is_enough(
    r#"<rsl><content url="/"><license><prohibits type="usage">all</prohibits></license><license><permits type="usage">ai-train</permits></license></content></rsl>"#.to_owned(),
    [true, false, true]
)]
#[case::no_xml("not xml <".to_owned(), [false, false, false])]
fn an_rsl_file_refuses_what_none_of_its_licenses_permits(
    #[case] file: String,
    #[case] expected: [bool; 3],
) {
    assert_eq!(
        refusals(|purpose| rsl_refusing(file.as_bytes(), purpose)),
        expected
    );
}

#[test]
fn no_file_is_judged_against_search() {
    let rsl = license(r#"<prohibits type="usage">all</prohibits>"#);

    assert_eq!(
        (
            content_signals_refusing(&signals(&[("search", false)]), Purpose::Search),
            tdmrep_file_refusing(
                CATALOG_RESERVED.as_bytes(),
                &url("https://example.test/catalog"),
                Purpose::Search
            ),
            rsl_refusing(rsl.as_bytes(), Purpose::Search),
        ),
        (None, None, None)
    );
}

#[rstest]
#[case::a_tdmrep_meta(r#"<head><meta name="tdm-reservation" content="1"></head>"#, true)]
#[case::a_tdmrep_meta_in_any_case(
    r#"<head><META NAME="TDM-Reservation" content=" 1"></head>"#,
    true
)]
#[case::a_tdmrep_meta_not_reserving(
    r#"<head><meta name="tdm-reservation" content="0"></head>"#,
    false
)]
#[case::no_markup("<p>tdm-reservation</p>", false)]
fn a_documents_meta_reserves_its_rights(#[case] html: &str, #[case] reserved: bool) {
    let (signals, _) = read_markup(&[], html, &url("https://example.test/a/b"));

    assert_eq!(refusals(|purpose| signals.refusing(purpose)), [reserved; 3]);
}

/// An RSL `Link` header field value naming `target`.
fn rsl_link(target: &str) -> String {
    format!(r#"<{target}>; rel="license"; type="application/rsl+xml""#)
}

#[rstest]
#[case::a_link_header(
    vec![rsl_link("https://example.test/license.xml")],
    "",
    Some("https://example.test/license.xml")
)]
#[case::a_relative_link_header_among_others(
    vec![format!("<style.css>; rel=stylesheet, {}", rsl_link("/license.xml"))],
    "",
    Some("https://example.test/license.xml")
)]
#[case::a_license_of_another_type(vec![r#"</license.html>; rel="license""#.to_owned()], "", None)]
#[case::a_link_element(
    vec![],
    r#"<head><base href="/base/"><link rel="license" type="application/rsl+xml" href="license.xml"></head>"#,
    Some("https://example.test/base/license.xml")
)]
#[case::the_header_first(
    vec![rsl_link("/header.xml")],
    r#"<head><link rel="license" type="application/rsl+xml" href="/element.xml"></head>"#,
    Some("https://example.test/header.xml")
)]
#[case::none(vec![], "<head></head>", None)]
fn a_document_names_its_rsl_license(
    #[case] links: Vec<String>,
    #[case] html: &str,
    #[case] expected: Option<&str>,
) {
    let at = url("https://example.test/a/b");
    let headers: Vec<(String, String)> = links
        .into_iter()
        .map(|value| ("link".to_owned(), value))
        .collect();
    let (signals, _) = read_markup(&headers, html, &at);

    assert_eq!(
        signals.license().map(String::from),
        expected.map(str::to_owned)
    );
}

/// A whole document whose head holds `head`, as a server sends one: its
/// tree is the streaming pass's stack.
fn document(head: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><title>A page</title>{head}</head><body><p>Text</p></body></html>"
    )
}

#[rstest]
#[case::a_tdmrep_meta(&[], r#"<meta name="tdm-reservation" content="1">"#)]
#[case::a_tdmrep_meta_in_any_case(&[], r#"<META NAME="TDM-Reservation" content=" 1">"#)]
#[case::a_tdmrep_meta_not_reserving(&[], r#"<meta name="tdm-reservation" content="0">"#)]
#[case::a_link_element_and_its_base(
    &[],
    r#"<base href="/base/"><link rel="stylesheet" href="s.css"><link rel="license" type="application/rsl+xml" href="license.xml">"#
)]
#[case::a_character_reference_in_its_href(
    &[],
    r#"<link rel="license" type="application/rsl+xml" href="/l&amp;i.xml">"#
)]
#[case::the_header_first(
    &[("link", r#"</header.xml>; rel="license"; type="application/rsl+xml""#)],
    r#"<link rel="license" type="application/rsl+xml" href="/element.xml">"#
)]
#[case::both(
    &[],
    r#"<base href="https://elsewhere.test/"><meta name="tdm-reservation" content="1"><link rel="license" type="application/rsl+xml" href="license.xml">"#
)]
fn a_documents_markup_signals_are_read_in_one_streaming_pass_as_its_tree_holds_them(
    #[case] pairs: &[(&str, &str)],
    #[case] head: &str,
) {
    let at = url("https://example.test/a/b");
    let html = document(head);
    let tree = {
        let work = CountedWork::default();
        let readers = UntouchedReaders::of(UntouchedDemand::default(), &work);
        let parsed = work.parse_html(Row::PageParses, &html);
        readers.use_signals(UseSignals::of(&headers(pairs)), (&html, &parsed), &at)
    };

    let (signals, parses) = read_markup(&headers(pairs), &html, &at);

    assert_eq!((signals, parses), (tree, 0));
}

#[test]
fn a_document_whose_tree_the_pass_cannot_vouch_for_is_parsed_once() {
    // No doctype: quirks mode, whose tree the pass does not follow.
    let html = r#"<head><meta name="tdm-reservation" content="1"></head>"#;

    let (signals, parses) = read_markup(&[], html, &url("https://example.test/"));

    assert_eq!(
        (signals.refusing(Purpose::Collect).is_some(), parses),
        (true, 1)
    );
}
