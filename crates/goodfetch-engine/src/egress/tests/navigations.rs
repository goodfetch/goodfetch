//! The scheme rules of a page's navigations through `egress`'s interface
//! (ADRs 0017, 0056, 0089 and 0120; spec #564's `file:` and `javascript:`
//! rules): a navigation a Client asks for may load a `file:` URL, or any
//! other document that reaches no network, only from the program's own code
//! or the CLI on an engine without accounts; a `javascript:` URL runs only
//! in a frame whose own origin the domain policy allows; `about:blank` is
//! everyone's; and every hop of a page's own requests to `file:` is
//! refused.
//!
//! Covers: story:4-5, story:4-6, error:egress_refused
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::egress::EgressRule;
use rstest::rstest;

use super::{policy, refused, verdict};
use crate::egress::{
    Destination, Navigator, Refused, Request, RequestKind, judge, judge_javascript, navigation,
};

/// The program's own code, or the CLI, on an engine without accounts.
const PROGRAM: Navigator = Navigator {
    acting_for_model: false,
    recipe_run: false,
    accounts: false,
};

const FILE: &str = "file:///home/user/report.html";

#[rstest]
#[case::a_file(FILE)]
#[case::data("data:text/html,<p>hi</p>")]
#[case::the_browsers_own_page("chrome://crash")]
#[case::another_about_page("about:srcdoc")]
#[case::gopher("gopher://127.0.0.1:6379/_INFO")]
fn the_programs_own_navigation_loads_a_document_of_the_machine(#[case] url: &str) {
    assert_eq!(navigation(url, PROGRAM), Ok(Destination::Local));
}

#[rstest]
#[case::a_client_acting_for_a_model(Navigator { acting_for_model: true, ..PROGRAM })]
#[case::a_recipe_run(Navigator { recipe_run: true, ..PROGRAM })]
#[case::an_engine_with_accounts(Navigator { accounts: true, ..PROGRAM })]
fn a_file_navigation_is_refused_for_anyone_but_the_program_on_an_engine_without_accounts(
    #[case] navigator: Navigator,
) {
    assert_eq!(
        navigation(FILE, navigator),
        Err(refused(FILE, EgressRule::Scheme, None))
    );
}

/// A navigation to an IPv6 address with a zone ID, a URL the WHATWG parser
/// refuses, is refused as `judge` refuses the request: a private address,
/// named, since a zone names a link (the SSRF table's `zone_id`, which the
/// Chromium layer met as a domain policy refusal).
#[test]
fn a_navigation_to_an_address_with_a_zone_id_is_refused_as_a_private_address() {
    let url = "http://[fe80::1%25eth0]/";

    assert_eq!(
        navigation(url, PROGRAM),
        Err(Refused {
            address: Some("fe80::1".parse().unwrap()),
            ..refused(url, EgressRule::PrivateAddress, None)
        })
    );
}

#[rstest]
#[case::http("http://scope.test/")]
#[case::https("https://scope.test/news")]
fn an_http_navigation_goes_to_the_network_where_the_interception_judges_it(#[case] url: &str) {
    assert_eq!(
        navigation(url, PROGRAM).map(|destination| destination == Destination::Network),
        Ok(true)
    );
}

#[test]
fn a_javascript_url_is_left_to_the_frame_rule() {
    assert_eq!(
        navigation("javascript:document.title='ran'", PROGRAM),
        Ok(Destination::JavaScript)
    );
}

#[test]
fn about_blank_loads_nothing_and_is_allowed() {
    assert_eq!(navigation("about:blank", PROGRAM), Ok(Destination::Blank));
}

#[rstest]
#[case::data("data:text/html,<p>hi</p>")]
#[case::gopher("gopher://127.0.0.1:6379/_INFO")]
#[case::the_browsers_own_page("chrome://history")]
#[case::ftp("ftp://scope.test/file")]
fn a_client_acting_for_a_model_is_refused_any_other_scheme(#[case] url: &str) {
    let model = Navigator {
        acting_for_model: true,
        ..PROGRAM
    };

    assert_eq!(
        navigation(url, model),
        Err(refused(url, EgressRule::Scheme, None))
    );
}

#[test]
fn a_url_that_does_not_parse_is_refused() {
    assert_eq!(
        navigation("http://[::1", PROGRAM),
        Err(refused("http://[::1", EgressRule::DomainPolicy, None))
    );
}

/// A main-frame hop the program's navigation allowed loads its `file:`
/// URL; a page's own request to `file:`, a redirect included, never does.
#[rstest]
#[case::the_programs_navigation(RequestKind::Navigation(PROGRAM), Ok(FILE.to_owned()))]
#[case::a_models_navigation(
    RequestKind::Navigation(Navigator { acting_for_model: true, ..PROGRAM }),
    Err(refused(FILE, EgressRule::Scheme, None))
)]
#[case::a_pages_own_request(RequestKind::Page, Err(refused(FILE, EgressRule::Scheme, None)))]
fn a_file_hop_is_judged_by_who_asked_for_it(
    #[case] kind: RequestKind,
    #[case] expected: Result<String, Refused>,
) {
    let request = Request { url: FILE, kind };

    assert_eq!(verdict(judge(&request, &policy(&["*"], &[]))), expected);
}

const SCRIPT: &str = "javascript:document.title='ran'";

#[test]
fn a_javascript_url_runs_in_a_frame_whose_origin_the_policy_allows() {
    assert_eq!(
        judge_javascript(
            SCRIPT,
            "https://www.scope.test/news",
            &policy(&["scope.test"], &[])
        ),
        Ok(())
    );
}

#[rstest]
#[case::not_allowed(
    "https://elsewhere.test/",
    EgressRule::DomainPolicy,
    Some("egress.allowed_domains")
)]
#[case::the_browsers_error_page("chrome-error://chromewebdata/", EgressRule::Scheme, None)]
#[case::a_blank_page("about:blank", EgressRule::Scheme, None)]
fn a_javascript_url_in_a_frame_whose_origin_the_policy_refuses_does_not_run(
    #[case] frame: &str,
    #[case] rule: EgressRule,
    #[case] setting: Option<&'static str>,
) {
    assert_eq!(
        judge_javascript(SCRIPT, frame, &policy(&["scope.test"], &[])),
        Err(refused(SCRIPT, rule, setting))
    );
}
