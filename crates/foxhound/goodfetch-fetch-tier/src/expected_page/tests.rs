//! Covers: story:5-137, setting:session.page_check
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;

use goodfetch_counted_work::Row;
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};
use rstest::rstest;

use super::{Verdict, judge};

const URL: &str = "http://shop.test/reports/2";

/// An app shell whose content only its script writes.
const SHELL: &str = r#"<html><body><main id="app"></main><script>document.getElementById("app").innerHTML = "<h1>Reports</h1>";</script></body></html>"#;

/// An app shell marked `noindex` by its robots `meta` element.
const NOINDEXED: &str = r#"<html><head><meta name="robots" content="noindex"></head><body><main id="app"></main></body></html>"#;

/// A page whose own markup shows its records and a login form.
const LISTING: &str = r#"<html><body><main id="records"><article>One</article></main><form id="login"></form></body></html>"#;

fn css(selector: &str) -> PageMatch {
    PageMatch::Css {
        selector: selector.to_owned(),
    }
}

fn glob(pattern: &str) -> PageMatch {
    PageMatch::Glob {
        pattern: pattern.to_owned(),
    }
}

fn check(login_page: Option<PageMatch>, expected_page: Option<PageMatch>) -> PageCheck {
    PageCheck {
        login_page,
        expected_page,
    }
}

/// A document is judged as the browser judges it where its markup alone
/// decides: its URL against a glob or a regular expression, an element a
/// CSS selector finds in its markup, the login page first. An element only
/// a script writes is not in the markup, so such a page is not the
/// expected page; a match the markup cannot decide exactly, an `XPath`
/// expression or a body text, or a pattern or selector the tier cannot
/// read, leaves the page unjudged, for the browser.
#[rstest]
#[case::expected_by_css(LISTING, check(None, Some(css("#records article"))), Verdict::Expected)]
#[case::expected_by_glob(SHELL, check(None, Some(glob("**/reports/*"))), Verdict::Expected)]
#[case::no_expected_page_passes(LISTING, check(Some(glob("**/login")), None), Verdict::Expected)]
#[case::written_by_script(SHELL, check(None, Some(css("#app h1"))), Verdict::NotExpected)]
#[case::another_url(LISTING, check(None, Some(PageMatch::Regex { pattern: "/catalog/".to_owned() })), Verdict::NotExpected)]
#[case::login_page_first(
    LISTING,
    check(Some(css("form#login")), Some(css("#records article"))),
    Verdict::LoginPage
)]
#[case::by_xpath(LISTING, check(None, Some(PageMatch::Xpath { selector: "//main".to_owned() })), Verdict::Unjudged)]
#[case::by_text(LISTING, check(None, Some(PageMatch::Text { text: "One".to_owned() })), Verdict::Unjudged)]
#[case::login_page_by_text(
    LISTING,
    check(Some(PageMatch::Text { text: "Sign in".to_owned() }), Some(css("#records article"))),
    Verdict::Unjudged
)]
#[case::selector_unread(LISTING, check(None, Some(css("main >>> article"))), Verdict::Unjudged)]
fn a_document_is_judged_where_its_markup_decides(
    #[case] html: &str,
    #[case] check: PageCheck,
    #[case] verdict: Verdict,
) {
    assert_eq!(
        judge(
            &check,
            URL,
            html,
            &BTreeMap::new(),
            &goodfetch_counted_work::CountedWork::default()
        )
        .0,
        verdict
    );
}

/// A page that is not its expected page is marked `noindex` by its robots
/// `meta` element or its `X-Robots-Tag` header, its markup parsed once for
/// its check and its robots `meta` elements together, and not at all when
/// neither needs it.
#[rstest]
#[case::by_meta_after_a_selector(NOINDEXED, check(None, Some(css("#app h1"))), &[], true, 1)]
#[case::by_meta_after_a_glob(NOINDEXED, check(None, Some(glob("**/catalog/*"))), &[], true, 1)]
#[case::not_marked(SHELL, check(None, Some(glob("**/catalog/*"))), &[], false, 1)]
#[case::by_header(SHELL, check(None, Some(glob("**/catalog/*"))), &[("x-robots-tag", "noindex")], true, 0)]
#[case::the_expected_page(NOINDEXED, check(None, Some(glob("**/reports/*"))), &[], false, 0)]
fn a_page_judged_alone_is_parsed_once_for_its_noindex_mark(
    #[case] html: &str,
    #[case] check: PageCheck,
    #[case] headers: &[(&str, &str)],
    #[case] noindex: bool,
    #[case] parses: u64,
) {
    let work = goodfetch_counted_work::CountedWork::default();
    let headers = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(judge(&check, URL, html, &headers, &work).1, noindex);
    assert_eq!(work.read(Row::PageParses), parses);
}
