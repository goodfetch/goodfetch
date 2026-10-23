//! The page suite (spec #577, tickets #695, #696, #697, #698, #699, #702 and #703; spec #580,
//! ticket #783) against an Attached engine of Collie's executable, given by path, on
//! the local Proving ground (ADR 0094): its browser from a Browser cache in
//! the test's own folder, which its environment names (ADR 0059).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProtocolSuite, ProvingGround, run_protocol_suite,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::result_document::{CapabilityResult, CaseOutcome, Outcome};
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, TestEnv, Tool, browser_cache, tools};

use crate::proving_ground::LocalCopy;

/// Covers: story:5-61, story:5-60, story:5-62, operation:run.new_session, operation:session.new_page, operation:session.close, error:closed, error:setting_not_applicable, error:capability_unavailable, error:run_ended, story:5-23, story:5-24, story:5-26, story:5-56, story:5-11, story:5-2, story:5-3, story:5-10, story:5-18, operation:run.new_page, operation:page.goto, operation:page.reload, operation:page.go_back, operation:page.go_forward, operation:locator.click, operation:locator.fill, operation:locator.press, error:navigation_failed, error:navigation_interrupted, story:5-29, story:5-30, story:5-31, error:too_many_redirects, error:response_truncated, event:page.download, story:6-1, story:6-8, story:6-12, operation:page.formats, story:5-40, story:5-41, story:5-42, operation:page.route, operation:session.route, operation:route.continue, operation:route.fallback, operation:route.fulfill, operation:route.abort, operation:route.fetch, error:page_handler_failed, error:route_already_handled, error:egress_refused, story:5-46, story:5-48, operation:page.on, operation:response.body, operation:page.wait_for_request_finished, error:body_unavailable, event:page.console, event:page.page_error, event:page.request, event:page.request_finished, event:page.request_failed, event:page.response, event:page.web_socket, event:web_socket.frame_sent, event:web_socket.frame_received, event:web_socket.close, event:page.load, event:page.dom_content_loaded
#[test]
fn the_page_suite_is_proved_against_collies_attached_engine() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed());
    let proving_ground = ProvingGround::local(copy.address());
    let mut environment = env.env().clone();
    environment.insert(
        BROWSERS_PATH_VARIABLE.to_owned(),
        browser_cache(&env).unwrap().display().to_string(),
    );
    let target = EngineTarget {
        executable: tools().unwrap().path(Tool::Collie).unwrap().to_path_buf(),
        mode: EngineMode::Attached,
        environment,
        folder: env.folder().to_path_buf(),
    };

    let document = run_protocol_suite(&ProtocolSuite::page(), &target, &proving_ground).unwrap();
    crate::keep(&document, "page-collie-attached");

    let cases: Vec<(&str, CaseOutcome)> = document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .map(|case| (case.facts().id.as_str(), case.outcome()))
        .collect();
    assert_eq!(
        (
            document
                .capabilities()
                .iter()
                .map(|capability| (capability.name(), capability.outcome()))
                .collect::<Vec<_>>(),
            document.header().target.product,
        ),
        (
            vec![
                ("sessions.default", Outcome::Proved),
                ("sessions", Outcome::Proved),
                ("navigation", Outcome::Proved),
                ("navigation.back_forward_cache", Outcome::Proved),
                ("navigation.failures", Outcome::Proved),
                ("navigation.redirects", Outcome::Proved),
                ("navigation.cut_documents", Outcome::Proved),
                ("navigation.decoding", Outcome::Proved),
                ("navigation.downloads", Outcome::Proved),
                ("formats", Outcome::Proved),
                ("launch", Outcome::Proved),
                ("locators.guard", Outcome::Proved),
                ("locators.input", Outcome::Proved),
                ("routes", Outcome::Proved),
                ("routes.interception_order", Outcome::Proved),
                ("events", Outcome::Proved),
                ("bodies", Outcome::Proved),
            ],
            Some(Product::Collie),
        ),
        "{cases:?} {:?}",
        document
            .capabilities()
            .iter()
            .flat_map(CapabilityResult::cases)
            .flat_map(|case| case.facts().entries.iter().flat_map(|entry| entry
                .checks
                .iter()
                .filter(|check| !check.passed)
                .map(|check| check.name.clone())))
            .collect::<Vec<_>>()
    );
}
