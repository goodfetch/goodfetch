use goodfetch_cdp::RawEvent;
use serde_json::{Value, json};

use super::Navigation;

/// The page's main frame.
const FRAME: &str = "main";

fn event(method: &str, params: Value) -> RawEvent {
    RawEvent {
        method: method.to_owned(),
        params,
    }
}

fn stopped() -> RawEvent {
    event("Page.frameStoppedLoading", json!({ "frameId": FRAME }))
}

fn requested() -> RawEvent {
    event(
        "Page.frameRequestedNavigation",
        json!({
            "frameId": FRAME,
            "reason": "anchorClick",
            "url": "https://site.test/b",
            "disposition": "currentTab",
        }),
    )
}

fn started() -> RawEvent {
    event(
        "Page.frameStartedNavigating",
        json!({
            "frameId": FRAME,
            "url": "https://site.test/b",
            "loaderId": "loader-b",
            "navigationType": "differentDocument",
        }),
    )
}

/// A `frameStoppedLoading` of the load before a gesture's navigation, read
/// once the watch started, never ends the wait for the navigation the
/// gesture requested, after any of the events: the wait reads them one at
/// a time, and the browser's start of the navigation can come after the
/// renderer's request, which can come before the stop or after it (#1766).
#[rstest::rstest]
#[case::before_the_request(vec![stopped(), requested(), started()])]
#[case::after_the_request(vec![requested(), stopped(), started()])]
fn a_stop_before_the_requested_navigation_started_does_not_end_it(#[case] events: Vec<RawEvent>) {
    let mut navigation = Navigation::of(FRAME.to_owned());

    let ended: Vec<bool> = events
        .iter()
        .map(|event| {
            navigation.feed(event);
            navigation.requested_ended()
        })
        .collect();

    assert_eq!(ended, [false, false, false]);
}

/// The main frame stopping once the requested navigation started ends it.
#[test]
fn a_stop_after_the_requested_navigation_started_ends_it() {
    let mut navigation = Navigation::of(FRAME.to_owned());

    for event in [requested(), started(), stopped()] {
        navigation.feed(&event);
    }

    assert!(navigation.requested_ended());
}

/// The main frame's `beforeunload` dialog, as the browser opens it before
/// it starts the requested navigation.
fn before_unload() -> RawEvent {
    event(
        "Page.javascriptDialogOpening",
        json!({
            "url": "https://site.test/",
            "frameId": FRAME,
            "message": "",
            "type": "beforeunload",
            "hasBrowserHandler": false,
            "defaultPrompt": "",
        }),
    )
}

/// The main frame's dialog closing, accepted or not.
fn dialog_closed(accepted: bool) -> RawEvent {
    event(
        "Page.javascriptDialogClosed",
        json!({ "frameId": FRAME, "result": accepted, "userInput": "" }),
    )
}

/// The page's `beforeunload` dialog dismissed cancels the navigation the
/// gesture requested, which ends the wait for it with the page left as it
/// was; accepted, the navigation goes on, and the wait with it.
#[rstest::rstest]
#[case::dismissed(false, true)]
#[case::accepted(true, false)]
fn a_before_unload_dialog_dismissed_ends_the_requested_navigation(
    #[case] accepted: bool,
    #[case] ended: bool,
) {
    let mut navigation = Navigation::of(FRAME.to_owned());

    for event in [requested(), before_unload(), dialog_closed(accepted)] {
        navigation.feed(&event);
    }

    assert_eq!(navigation.requested_ended(), ended);
}

/// A URL the URL Standard reads as `javascript:` runs a script in the
/// frame, in every spelling the engine's egress check accepts: tabs and
/// newlines anywhere and leading spaces dropped, its scheme in any case
/// (ADR 0067; ticket #631).
#[rstest::rstest]
#[case::plain("javascript:void(0)", true)]
#[case::its_scheme_in_capitals("JavaScript:void(0)", true)]
#[case::a_tab_in_its_scheme("java\tscript:void(0)", true)]
#[case::a_newline_in_its_scheme("java\nscript:void(0)", true)]
#[case::leading_spaces("  javascript:void(0)", true)]
#[case::a_network_url_naming_it("https://site.test/javascript:void(0)", false)]
fn a_javascript_url_is_told_apart_as_the_egress_check_reads_it(
    #[case] url: &str,
    #[case] runs: bool,
) {
    assert_eq!(super::runs_a_script(url), runs);
}
