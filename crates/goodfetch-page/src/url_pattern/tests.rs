//! The glob matcher against Playwright's own glob cases (its
//! `interception.spec.ts` "should work with glob", without a base URL),
//! each expected value the one Playwright 1.64.1's `urlMatches` gives, and
//! the rules every glob keeps, as properties.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::CallError;
use goodfetch_protocol::page::PatternConstruct;
use proptest::prelude::*;
use rstest::rstest;

use super::UrlPattern;

/// Covers: story:5-43
#[rstest]
#[case::double_star_then_extension("**/*.js", "https://localhost:8080/foo.js", true)]
#[case::another_extension("**/*.css", "https://localhost:8080/foo.js", false)]
#[case::single_star_stays_in_a_segment("*.js", "https://localhost:8080/foo.js", false)]
#[case::scheme_then_double_star("https://**/*.js", "https://localhost:8080/foo.js", true)]
#[case::literal_url(
    "http://localhost:8080/simple/path.js",
    "http://localhost:8080/simple/path.js",
    true
)]
#[case::braces_first("**/{a,b}.js", "https://localhost:8080/a.js", true)]
#[case::braces_second("**/{a,b}.js", "https://localhost:8080/b.js", true)]
#[case::braces_neither("**/{a,b}.js", "https://localhost:8080/c.js", false)]
#[case::extension_braces_jpg("**/*.{png,jpg,jpeg}", "https://localhost:8080/c.jpg", true)]
#[case::extension_braces_jpeg("**/*.{png,jpg,jpeg}", "https://localhost:8080/c.jpeg", true)]
#[case::extension_braces_png("**/*.{png,jpg,jpeg}", "https://localhost:8080/c.png", true)]
#[case::extension_braces_css("**/*.{png,jpg,jpeg}", "https://localhost:8080/c.css", false)]
#[case::trailing_star("foo*", "foo.js", true)]
#[case::trailing_star_stops_at_a_slash("foo*", "foo/bar.js", false)]
#[case::star_after_a_path_stops_at_a_slash(
    "http://localhost:3000/signin-oidc*",
    "http://localhost:3000/signin-oidc/foo",
    false
)]
#[case::star_after_a_path(
    "http://localhost:3000/signin-oidc*",
    "http://localhost:3000/signin-oidcnice",
    true
)]
#[case::brackets_are_no_class(
    "**/three-columns/settings.html?**id=[a-z]**",
    "http://mydomain:8080/blah/blah/three-columns/settings.html?id=settings-e3c58efe-02e9-44b0-97ac-dd138100cf7c&blah",
    false
)]
#[case::origin_gains_its_slash("http://playwright.dev", "http://playwright.dev/", true)]
#[case::query_after_an_origin("http://playwright.dev?a=b", "http://playwright.dev/?a=b", true)]
#[case::star_in_the_scheme("h*://playwright.dev", "http://playwright.dev/", true)]
#[case::star_in_the_host("http://*.playwright.dev?x=y", "http://api.playwright.dev/?x=y", true)]
#[case::double_star_on_both_sides("**/foo/**", "http://playwright.dev/foo/bar", true)]
#[case::question_mark_starts_the_query(
    "http://localhost:8080/?imple/path.js",
    "http://localhost:8080/Simple/path.js",
    false
)]
#[case::question_mark_in_the_host("http://playwright.?ev", "http://playwright.dev/", false)]
#[case::question_mark_in_the_host_against_a_query(
    "http://playwright.?ev",
    "http://playwright.dev/?",
    false
)]
#[case::question_marks_are_no_wildcard(
    "http://playwright.dev/f??",
    "http://playwright.dev/foo",
    false
)]
#[case::question_marks_match_themselves(
    "http://playwright.dev/f??",
    "http://playwright.dev/f??",
    true
)]
#[case::escaped_question_mark_after_the_host(
    "http://playwright.dev\\?x=y",
    "http://playwright.dev/?x=y",
    true
)]
#[case::escaped_question_mark_after_the_path(
    "http://playwright.dev/\\?x=y",
    "http://playwright.dev/?x=y",
    true
)]
#[case::double_star_segment_matches_none("https://foo/**/bar.js", "https://foo/bar.js", true)]
#[case::two_double_star_segments_match_none("https://foo/**/**/bar.js", "https://foo/bar.js", true)]
#[case::about_blank("about:blank", "about:blank", true)]
#[case::about_star("about:*", "about:blank", true)]
#[case::about_star_is_a_prefix("about:*", "notabout:blank", false)]
#[case::data_star_stops_at_a_slash("data:*", "data:text/html,hello", false)]
#[case::double_star_slash_query("**/?a=b", "https://localhost:3000/?a=b", true)]
#[case::double_star_query("**?a=b", "https://localhost:3000/?a=b", true)]
#[case::double_star_value("**=b", "https://localhost:3000/?a=b", true)]
#[case::host_in_any_case("http://PLAYWRIGHT.dev/abcd", "http://playwright.dev/abcd", true)]
#[case::path_in_its_own_case("http://playwright.dev/ABCD", "http://playwright.dev/abcd", false)]
#[case::wildcard_host_in_any_case("http://*.PLAYWRIGHT.dev/x", "http://api.playwright.dev/x", true)]
#[case::empty_glob("", "https://anything.test/at/all", true)]
#[case::double_star_alone("**", "https://anything.test/at/all", true)]
#[case::comma_outside_braces("**/a,b", "https://x.test/a,b", true)]
#[case::escaped_star("**/\\*.js", "https://x.test/*.js", true)]
#[case::escaped_star_is_no_wildcard("**/\\*.js", "https://x.test/a.js", false)]
#[case::brackets_match_themselves("**/[a-z].js", "https://x.test/[a-z].js", true)]
#[case::brackets_are_no_range("**/[a-z].js", "https://x.test/a.js", false)]
#[case::trailing_backslash("**/a\\", "https://x.test/a\\", true)]
#[case::space_is_percent_encoded("https://x.test/a b", "https://x.test/a%20b", true)]
#[case::non_ascii_path_is_percent_encoded(
    "https://x.test/caf\u{e9}/*",
    "https://x.test/caf%C3%A9/menu",
    true
)]
#[case::default_port_dropped("https://x.test:443/*", "https://x.test/page", true)]
#[case::dot_segments_resolved("https://x.test/a/../b", "https://x.test/b", true)]
fn a_glob_matches_the_urls_playwright_matches(
    #[case] glob: &str,
    #[case] url: &str,
    #[case] matches: bool,
) {
    let pattern = UrlPattern::glob(glob).unwrap();

    assert_eq!(pattern.matches(url), matches);
}

/// Covers: story:5-43
#[rstest]
#[case::unmatched_opening_brace("**/{a,b", PatternConstruct::UnmatchedBrace)]
#[case::unmatched_closing_brace("**/a}", PatternConstruct::UnmatchedBrace)]
#[case::nested_braces("**/{a,{b,c}}", PatternConstruct::NestedBraces)]
fn a_glob_playwright_refuses_is_refused(#[case] glob: &str, #[case] construct: PatternConstruct) {
    let refused = UrlPattern::glob(glob).unwrap_err();

    assert_eq!(refused.construct(), construct);
}

/// Covers: error:pattern_unsupported
#[test]
fn a_refused_pattern_raises_pattern_unsupported() {
    let refused = UrlPattern::regex("a(?=b)").unwrap_err();

    assert_eq!(CallError::from(refused).code(), "pattern_unsupported");
}

/// A URL's path segment: no `/`, `?`, `#`, `%`, glob character or dot
/// segment.
fn segment() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9._~-]{0,11}"
}

/// An `https` URL on a lowercase host, with one to four path segments: its
/// host, its segments and the URL.
fn url() -> impl Strategy<Value = (String, Vec<String>, String)> {
    (
        "[a-z]{1,10}\\.test",
        proptest::collection::vec(segment(), 1..5),
    )
        .prop_map(|(host, segments)| {
            let url = format!("https://{host}/{}", segments.join("/"));
            (host, segments, url)
        })
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:5-43
    #[test]
    fn a_double_star_matches_every_url((_, _, url) in url()) {
        let pattern = UrlPattern::glob("**").unwrap();

        prop_assert!(pattern.matches(&url));
    }

    /// Covers: story:5-43
    #[test]
    fn a_double_star_then_the_last_segment_matches_the_url((_, segments, url) in url()) {
        let glob = format!("**/{}", segments.last().unwrap());
        let pattern = UrlPattern::glob(&glob).unwrap();

        prop_assert!(pattern.matches(&url));
    }

    /// Covers: story:5-43
    #[test]
    fn a_single_star_matches_one_path_segment_only((host, segments, url) in url()) {
        let glob = format!("https://{host}/*");
        let pattern = UrlPattern::glob(&glob).unwrap();

        prop_assert_eq!(pattern.matches(&url), segments.len() == 1);
    }

    /// Covers: story:5-43
    #[test]
    fn a_glob_of_a_url_matches_that_url_and_not_a_longer_one((_, _, url) in url()) {
        let longer = url.clone() + "x";
        let pattern = UrlPattern::glob(&url).unwrap();

        prop_assert!(pattern.matches(&url));
        prop_assert!(!pattern.matches(&longer));
    }
}
