//! Which robots directives mark a document `noindex` (ADR 0035).

use std::collections::BTreeMap;

use rstest::rstest;

use super::marked;

/// Each header value and robots `meta` content, and whether it marks the
/// document `noindex`.
#[rstest]
#[case::no_directive(None, &[], false)]
#[case::the_header(Some("noindex"), &[], true)]
#[case::the_header_among_others(Some("nofollow, NoIndex"), &[], true)]
#[case::none_is_noindex_and_nofollow(Some("none"), &[], true)]
#[case::a_header_for_one_crawler_only(Some("googlebot: noindex"), &[], false)]
#[case::the_header_without_noindex(Some("nofollow, noarchive"), &[], false)]
#[case::the_meta_element(None, &["noindex, nofollow"], true)]
#[case::a_second_meta_element(None, &["index", " noindex "], true)]
#[case::a_meta_element_without_noindex(None, &["index, follow"], false)]
#[case::unavailable_after_is_no_noindex(Some("unavailable_after: 2026-01-01"), &[], false)]
fn a_noindex_or_none_directive_marks_the_document(
    #[case] header: Option<&str>,
    #[case] robots: &[&str],
    #[case] expected: bool,
) {
    let headers: BTreeMap<String, String> = header
        .map(|value| ("x-robots-tag".to_owned(), value.to_owned()))
        .into_iter()
        .collect();
    let robots: Vec<String> = robots.iter().map(|&content| content.to_owned()).collect();

    assert_eq!(marked(&headers, &robots), expected);
}
