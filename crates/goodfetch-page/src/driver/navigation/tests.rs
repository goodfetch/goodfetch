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
/// gesture requested.
#[test]
fn a_stop_before_the_requested_navigation_started_does_not_end_it() {
    let mut navigation = Navigation::of(FRAME.to_owned());

    for event in [stopped(), requested(), started()] {
        navigation.feed(&event);
    }

    assert!(!navigation.requested_ended());
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
