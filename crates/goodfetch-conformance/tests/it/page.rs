//! The page suite (spec #577, tickets #695, #696, #697, #698, #699, #702, #703, #704, #706, #707, #708, #713, #715, #717, #719, #705, #710 and #716; spec #580,
//! tickets #783, #789 and #2402) against an Attached engine of Collie's executable, given by path, on
//! the local Proving ground (ADR 0094): its browser from a Browser cache in
//! the test's own folder, which its environment names (ADR 0059).
//!
//! The suite runs in parts, one test per Feature group, each on its own
//! Proving ground copy and engine, so each part keeps well within nextest's
//! kill on its own (ADR 0064: a longer limit is no fix). Together the parts
//! take every capability of the suite's case list once, so each Feature is
//! still proved by its cases, and each part keeps its own document.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProtocolSuite, ProvingGround, run_protocol_suite,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::result_document::{CapabilityResult, CaseOutcome, Outcome};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround as LocalCopy, TestEnv, Tool, browser_cache,
    skipped_without_gui_session, tools,
};

/// One part of the page suite: its name, which names its kept document,
/// and the capabilities it runs, in the case list's order.
type Part = (&'static str, &'static [&'static str]);

const SESSIONS: Part = (
    "sessions",
    &[
        "sessions.default",
        "sessions",
        "sessions.page_check",
        "sessions.blocked",
        "sessions.proxy",
    ],
);
const PROXY_LIMITS: Part = ("proxy_limits", &["sessions.proxy_limits"]);
const NAVIGATION: Part = (
    "navigation",
    &[
        "navigation",
        "navigation.back_forward_cache",
        "navigation.failures",
        "navigation.redirects",
        "navigation.cut_documents",
        "navigation.decoding",
        "navigation.downloads",
    ],
);
const FORMATS: Part = ("formats", &["formats"]);
const FORMATS_EXTRACTOR: Part = ("formats-extractor", &["formats.extractor"]);
const LAUNCH: Part = ("launch", &["launch"]);
const LOCATORS: Part = ("locators", &["locators.guard", "locators.input"]);
const ROUTES: Part = ("routes", &["routes", "routes.interception_order"]);
const CLEAN_UP_BLOCKING: Part = (
    "clean-up-blocking",
    &[
        "clean_up.filter_lists",
        "clean_up.never_block",
        "clean_up.block_resources",
    ],
);
const CLEAN_UP_BLOCKING_OFF: Part = ("clean-up-blocking-off", &["clean_up.blocking_off"]);
const CLEAN_UP_HIDING: Part = ("clean-up-hiding", &["clean_up.hiding"]);
const CONSENT: Part = ("consent", &["clean_up.consent", "clean_up.consent_walls"]);
const EVENTS: Part = ("events", &["events", "bodies"]);
const PAGE_STATES: Part = ("page-states", &["page_states.service_workers"]);
const PAGE_REQUEST: Part = (
    "page-request",
    &[
        "page_request",
        "page_request.fetch_edge",
        "page_request.decoding",
    ],
);
const DIALOGS: Part = ("dialogs", &["dialogs"]);
const POPUPS: Part = ("popups", &["popups"]);
const PAGE_FUNCTIONS: Part = ("page-functions", &["page_functions"]);
const READ: Part = ("read", &["read"]);
const WAITS: Part = ("waits", &["waits"]);
const ROBOTS_TXT: Part = ("robots-txt", &["robots_txt"]);

/// Every part.
const PARTS: [Part; 21] = [
    SESSIONS,
    PROXY_LIMITS,
    NAVIGATION,
    FORMATS,
    FORMATS_EXTRACTOR,
    LAUNCH,
    LOCATORS,
    ROUTES,
    CLEAN_UP_BLOCKING,
    CLEAN_UP_BLOCKING_OFF,
    CLEAN_UP_HIDING,
    CONSENT,
    EVENTS,
    PAGE_STATES,
    PAGE_REQUEST,
    DIALOGS,
    POPUPS,
    PAGE_FUNCTIONS,
    READ,
    WAITS,
    ROBOTS_TXT,
];

#[test]
fn the_page_suites_parts_take_each_of_its_capabilities_once() {
    let mut declared: Vec<String> = ProtocolSuite::page()
        .case_list()
        .capabilities
        .iter()
        .map(|capability| capability.name.clone())
        .collect();
    declared.sort();
    let mut taken: Vec<&str> = PARTS
        .iter()
        .flat_map(|(_, capabilities)| capabilities.iter().copied())
        .collect();
    taken.sort_unstable();

    assert_eq!(taken, declared);
}

/// Runs `part` of the page suite against Collie's Attached engine on a
/// Proving ground copy of its own, keeps its document, and asserts that
/// each of its capabilities, and only those, is proved.
fn prove(part: Part) {
    prove_in(&TestEnv::new(), part, part.0);
}

/// [`prove`] in `env`, its document kept as `name`'s.
fn prove_in(env: &TestEnv, (_, capabilities): Part, name: &str) {
    let copy = LocalCopy::start(env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let mut environment = env.env().clone();
    environment.insert(
        BROWSERS_PATH_VARIABLE.to_owned(),
        browser_cache(env).unwrap().display().to_string(),
    );
    let target = EngineTarget {
        executable: tools().unwrap().path(Tool::Collie).unwrap().to_path_buf(),
        mode: EngineMode::Attached,
        environment,
        folder: env.folder().to_path_buf(),
    };
    let suite = ProtocolSuite::page()
        .with_capabilities(capabilities)
        .unwrap();

    let document = run_protocol_suite(&suite, &target, &proving_ground).unwrap();
    crate::keep(&document, &format!("page-collie-attached-{name}"));

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
            capabilities
                .iter()
                .map(|capability| (*capability, Outcome::Proved))
                .collect(),
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

/// Covers: story:5-61, story:5-60, story:5-62, operation:run.new_session, operation:session.new_page, operation:session.close, error:closed, error:setting_not_applicable, error:capability_unavailable, error:run_ended, story:5-79, setting:session.page_check, error:logged_out, error:page_check_failed, operation:run.new_page, operation:page.goto, error:setting_invalid, story:5-88, story:5-89, error:proxy_unavailable, error:proxy_auth_failed, setting:proxy.server, setting:proxy.password, error:setting_locked, story:5-91, story:8-63, story:8-58, story:8-59, error:blocked, operation:locator.click, operation:page.wait_for_url, operation:page.evaluate, operation:page.request, operation:session.pages, setting:session.source, operation:run.start
#[test]
fn the_page_suite_proves_sessions_against_collies_attached_engine() {
    prove(SESSIONS);
}

/// Covers: story:5-90, setting:politeness.proxy.concurrency, setting:politeness.proxy.rate, setting:politeness.sharing, setting:proxy.server, setting:proxy.username, error:proxy_unavailable, operation:run.new_page, operation:page.goto
#[test]
fn the_page_suite_proves_proxy_limits_against_collies_attached_engine() {
    prove(PROXY_LIMITS);
}

/// Covers: story:5-61, story:5-23, story:5-24, story:5-26, operation:run.new_page, operation:page.goto, operation:page.reload, operation:page.go_back, operation:page.go_forward, operation:page.wait_for_url, operation:page.wait_for_response, error:navigation_failed, error:navigation_interrupted, story:5-29, story:5-30, story:5-31, error:too_many_redirects, error:response_truncated, event:page.download, story:3-52, setting:timeout.navigation, error:deadline_exceeded
#[test]
fn the_page_suite_proves_navigation_against_collies_attached_engine() {
    if skipped_without_gui_session(
        "its download case needs a navigation to a PDF to be a download, which the headless shell does not make it",
    ) {
        return;
    }
    prove(NAVIGATION);
}

/// Covers: story:6-1, story:6-8, story:6-12, story:6-9, story:6-10, story:6-14, story:6-2, story:6-3, story:6-4, story:6-5, story:6-6, story:6-7, operation:page.formats, operation:page.wait_for_function, operation:page.evaluate, operation:locator.wait_for, setting:filter_lists.ads
#[test]
fn the_page_suite_proves_formats_against_collies_attached_engine() {
    prove(FORMATS);
}

/// `formats`' output never depends on the host's time zone or locale
/// (ADR 0073): under `TZ=Pacific/Chatham` and `LANG=tr_TR.UTF-8`, the
/// engine's own environment, every format of the formats part still equals
/// what the oracle holds, value for value, as under the host's; its
/// `GOODFETCH_HOME` and Browser cache sit in the test's folder, whose path
/// holds a space and a non-ASCII letter. The Release suite runs it with the
/// Pre-merge suite's tests.
///
/// Covers: story:6-1, story:6-2, story:6-3, story:6-4, story:6-5, story:6-6, story:6-7, operation:page.formats
#[test]
fn the_page_suite_proves_formats_under_another_time_zone_and_locale() {
    let elsewhere = TestEnv::new()
        .with_time_zone("Pacific/Chatham")
        .unwrap()
        .with_locale("tr-TR")
        .unwrap();
    let folder = elsewhere.folder().display().to_string();
    assert!(
        folder.contains(' ') && !folder.is_ascii(),
        "the test's folder holds a space and a non-ASCII letter: {folder}"
    );

    prove_in(&elsewhere, FORMATS, "formats-chatham-tr");
}

/// Covers: story:6-11, story:6-1, story:6-2, story:6-3, setting:content.extractor, operation:page.formats, operation:run.new_page, operation:run.new_session, operation:session.new_page, error:setting_invalid, error:setting_not_applicable
#[test]
fn the_page_suite_proves_the_extractor_against_collies_attached_engine() {
    prove(FORMATS_EXTRACTOR);
}

/// Covers: story:5-56, story:5-11, operation:run.new_page, operation:page.goto
#[test]
fn the_page_suite_proves_launch_against_collies_attached_engine() {
    if skipped_without_gui_session(
        "its headless identity case reads the full build's identity, where the headless shell shows `HeadlessChrome`",
    ) {
        return;
    }
    prove(LAUNCH);
}

/// Covers: story:5-2, story:5-3, story:5-10, story:5-18, operation:locator.click, operation:locator.fill, operation:locator.press
#[test]
fn the_page_suite_proves_locators_against_collies_attached_engine() {
    prove(LOCATORS);
}

/// Covers: operation:page.goto, story:5-40, story:5-41, story:5-42, operation:page.route, operation:session.route, operation:route.continue, operation:route.fallback, operation:route.fulfill, operation:route.abort, operation:route.fetch, error:page_handler_failed, error:route_already_handled, error:egress_refused, error:header_value_invalid, error:closed
#[test]
fn the_page_suite_proves_routes_against_collies_attached_engine() {
    prove(ROUTES);
}

/// Covers: error:navigation_failed, story:5-42, operation:page.route, operation:route.continue, story:5-94, story:5-95, story:5-96, setting:filter_lists.ads, setting:filter_lists.trackers, setting:filter_lists.annoyances, setting:filter_lists.first_party_scripts, setting:browser.block_resources, error:setting_invalid, operation:page.goto
#[test]
fn the_page_suite_proves_clean_up_blocking_against_collies_attached_engine() {
    prove(CLEAN_UP_BLOCKING);
}

/// Covers: error:navigation_failed, operation:page.route, story:5-99, setting:session.page_check, operation:run.new_session, operation:session.new_page, operation:page.goto, error:page_check_failed, operation:page.evaluate
#[test]
fn the_page_suite_proves_clean_up_blocking_off_against_collies_attached_engine() {
    prove(CLEAN_UP_BLOCKING_OFF);
}

/// Covers: operation:locator.click, operation:page.formats, story:5-98, setting:live_view.hiding, operation:page.goto
#[test]
fn the_page_suite_proves_clean_up_hiding_against_collies_attached_engine() {
    prove(CLEAN_UP_HIDING);
}

/// Covers: story:5-100, setting:consent.handling, story:5-101, error:blocked, operation:page.evaluate, setting:consent.domains, setting:consent.on_wall, error:setting_locked, error:capability_unavailable, story:12-26, operation:needs_you.list
#[test]
fn the_page_suite_proves_consent_against_collies_attached_engine() {
    prove(CONSENT);
}

/// Covers: error:response_truncated, error:egress_refused, story:5-46, story:5-48, operation:page.on, operation:response.body, operation:page.wait_for_request_finished, error:body_unavailable, event:page.console, event:page.page_error, event:page.request, event:page.request_finished, event:page.request_failed, event:page.response, event:page.web_socket, event:web_socket.frame_sent, event:web_socket.frame_received, event:web_socket.close, event:page.load, event:page.dom_content_loaded
#[test]
fn the_page_suite_proves_events_against_collies_attached_engine() {
    prove(EVENTS);
}

/// Covers: operation:run.new_session, operation:page.route, operation:session.route, event:page.request, event:page.response, story:5-49, story:5-50, setting:browser.service_workers, operation:page.wait_for_response, operation:page.wait_for_load_state, story:5-56, setting:browser.headless, operation:page.goto
#[test]
fn the_page_suite_proves_page_states_against_collies_attached_engine() {
    if skipped_without_gui_session(
        "its service-worker case reads the full build's window identity, where the headless shell shows `HeadlessChrome`",
    ) {
        return;
    }
    prove(PAGE_STATES);
}

/// Covers: operation:run.new_session, operation:session.new_page, error:navigation_failed, error:too_many_redirects, error:response_truncated, error:egress_refused, error:setting_invalid, operation:response.body, story:5-44, story:5-45, story:5-47, operation:page.request, error:header_value_invalid, error:response_limit_exceeded
#[test]
fn the_page_suite_proves_page_request_against_collies_attached_engine() {
    prove(PAGE_REQUEST);
}

/// Covers: operation:page.goto, operation:locator.click, error:page_handler_failed, story:5-52, story:5-53, operation:page.on_dialog, operation:dialog.accept, operation:dialog.dismiss, error:dialog_already_handled
#[test]
fn the_page_suite_proves_dialogs_against_collies_attached_engine() {
    prove(DIALOGS);
}

/// Covers: error:closed, operation:page.on, story:5-55, event:page.popup, operation:session.on, event:session.page, operation:session.pages, operation:page.close, event:page.close
#[test]
fn the_page_suite_proves_popups_against_collies_attached_engine() {
    prove(POPUPS);
}

/// Covers: operation:page.reload, story:5-34, story:5-35, operation:page.evaluate, operation:page.add_init_script, error:page_value_invalid, error:page_function_failed, operation:locator.evaluate_all
#[test]
fn the_page_suite_proves_page_functions_against_collies_attached_engine() {
    prove(PAGE_FUNCTIONS);
}

/// Covers: operation:page.read
#[test]
fn the_page_suite_proves_page_read_against_collies_attached_engine() {
    prove(READ);
}

/// Covers: story:5-19, story:5-20, story:5-21, story:5-22, operation:page.wait_for_url, operation:page.wait_for_request, operation:page.wait_for_response, operation:page.wait_for_load_state, operation:page.wait_for_function, operation:locator.wait_for_function, operation:locator.wait_for, operation:locator.wait_for_count, operation:page.evaluate, operation:locator.click, error:deadline_exceeded, setting:timeout.wait, story:5-43, error:pattern_unsupported, operation:page.route
#[test]
fn the_page_suite_proves_waits_against_collies_attached_engine() {
    prove(WAITS);
}

/// Covers: story:6-62, story:6-63, story:6-64, operation:run.new_page, operation:page.goto, operation:locator.click, operation:page.evaluate, operation:page.wait_for_url, operation:page.request, operation:run.new_session, operation:session.new_page, operation:session.close
#[test]
fn the_page_suite_proves_robots_txt_against_collies_attached_engine() {
    prove(ROBOTS_TXT);
}
