use std::cell::Cell;
use std::time::Duration;

use goodfetch_cdp::generated::page;
use goodfetch_cdp::{NetError, RawEvent, SessionId};
use goodfetch_protocol::page::WaitUntil;
use goodfetch_test_support::runtime;
use serde_json::{Value, json};

use super::{Arrived, Ended, Expected, Followed, Navigation};
use crate::driver::replay::{self, SESSION};

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

/// Where the replayed `page.goto` goes.
const URL: &str = "https://site.test/b";

/// The navigation `Page.navigate` named.
const LOADER: &str = "loader-b";

/// How many times the browser's order is replayed: the wait read the events
/// before the answer in each, never by chance.
const REPLAYS: usize = 64;

/// The main frame's stop of the load before the replayed navigation's.
fn stopped_message() -> Value {
    replay::event("Page.frameStoppedLoading", json!({ "frameId": FRAME }))
}

/// The browser's start of the navigation of `loader` in the main frame.
fn started_message(loader: &str) -> Value {
    replay::event(
        "Page.frameStartedNavigating",
        json!({
            "frameId": FRAME,
            "url": URL,
            "loaderId": loader,
            "navigationType": "differentDocument",
        }),
    )
}

/// The browser's error page for the navigation of `loader`, committed in
/// the main frame under its loader and loaded.
fn error_page_messages(loader: &str) -> Vec<Value> {
    vec![
        replay::event(
            "Page.frameNavigated",
            json!({
                "frame": {
                    "id": FRAME,
                    "loaderId": loader,
                    "url": "chrome-error://chromewebdata/",
                    "domainAndRegistry": "",
                    "securityOrigin": "://",
                    "mimeType": "text/html",
                    "secureContextType": "InsecureScheme",
                    "crossOriginIsolatedContextType": "NotIsolated",
                    "gatedAPIFeatures": [],
                },
                "type": "Navigation",
            }),
        ),
        replay::event(
            "Page.lifecycleEvent",
            json!({ "frameId": FRAME, "loaderId": loader, "name": "load", "timestamp": 1.0 }),
        ),
    ]
}

/// One replay of a `page.goto` whose `Page.navigate` the browser answers
/// with `navigated`, in one write after the events `before`, and after
/// which it sends the events `after` once a second of the paused clock
/// passed: how the wait ended, and whether the browser had sent `after`
/// by then.
async fn followed_once(
    before: &[Value],
    navigated: &Value,
    after: &[Value],
    wait_until: WaitUntil,
) -> (Followed, bool) {
    let ((connection, driver), mut browser) = replay::connected();
    let session = SessionId(SESSION.to_owned());
    let mut events = connection.subscribe(Some(&session));
    let mut navigation = Navigation::of(FRAME.to_owned());
    let after_sent = Cell::new(false);
    let (_, followed, ()) = tokio::join!(
        driver.run(),
        async {
            let command = async {
                let answered = connection
                    .send(
                        Some(&session),
                        &page::NavigateParams {
                            url: URL.to_owned(),
                            referrer: None,
                            transition_type: None,
                            frame_id: None,
                            referrer_policy: None,
                        },
                        Duration::from_secs(30),
                    )
                    .await
                    .expect("invariant: the played browser answers");
                Ok(super::navigated(URL, answered, false))
            };
            let followed = navigation
                .follow(command, &mut events, wait_until, || false)
                .await
                .expect("invariant: the command answered");
            let read_after = after_sent.get();
            drop((events, connection));
            (followed, read_after)
        },
        async {
            let command = browser.command().await;
            let mut messages = before.to_vec();
            messages.push(replay::answer(&command, navigated.clone()));
            assert!(browser.send(&messages).await, "the client left");
            tokio::time::sleep(Duration::from_secs(1)).await;
            after_sent.set(true);
            // The client may be gone: the wait ended without these.
            let _ = browser.send(after).await;
            browser.hold().await;
        }
    );
    followed
}

/// A `page.goto` decides how its navigation ended only once it read every
/// event the browser sent before `Page.navigate`'s answer, as the browser's
/// pipe orders them, whichever of the two the wait could poll first
/// (#1770):
/// - a failure the browser answers with the navigation's loader waits for
///   the error page it then commits under that loader, though the main
///   frame's stop of the load before came first: the navigation's start,
///   which the browser sent next, makes that stop an earlier load's;
/// - an answer within the document, and a failure with no loader, end at
///   the answer, whatever the browser sent before it.
#[rstest::rstest]
#[case::a_failure_with_its_loader_after_a_stop_of_the_load_before(
    vec![stopped_message(), started_message(LOADER)],
    json!({ "frameId": FRAME, "loaderId": LOADER, "errorText": "net::ERR_CONNECTION_REFUSED" }),
    error_page_messages(LOADER),
    Followed::Ended {
        outcome: Err(Ended::Failed(NetError::of("net::ERR_CONNECTION_REFUSED"))),
        expected: Some(Expected::Failed(
            LOADER.to_owned(),
            NetError::of("net::ERR_CONNECTION_REFUSED"),
        )),
    },
    true
)]
#[case::within_the_document_after_the_page_started_another_navigation(
    vec![started_message("loader-c")],
    json!({ "frameId": FRAME }),
    vec![],
    Followed::Ended { outcome: Ok(Arrived::Document(None)), expected: None },
    false
)]
#[case::a_failure_with_no_loader_after_a_stop(
    vec![stopped_message(), started_message("loader-c")],
    json!({ "frameId": FRAME, "errorText": "net::ERR_ABORTED" }),
    vec![],
    Followed::Ended { outcome: Err(Ended::Failed(NetError::Aborted)), expected: None },
    false
)]
fn a_goto_reads_the_events_sent_before_its_answer_before_it_decides(
    #[case] before: Vec<Value>,
    #[case] navigated: Value,
    #[case] after: Vec<Value>,
    #[case] expected: Followed,
    #[case] read_after: bool,
) {
    runtime().block_on(async {
        let mut differing = Vec::new();
        for _ in 0..REPLAYS {
            let followed = followed_once(&before, &navigated, &after, WaitUntil::Load).await;
            if followed.0 != expected || followed.1 != read_after {
                differing.push(followed);
            }
        }

        assert_eq!(differing, [], "replays that ended otherwise, of {REPLAYS}");
    });
}

/// The request of the main frame's document under `LOADER`.
const DOCUMENT_REQUEST: &str = "request-b";

/// The main frame's events for a document of `URL` the browser loaded,
/// answered `from_disk_cache` or not, whose load it reported finished with
/// `encoded` bytes counted, then fired `load`: the network's failure of a
/// body that broke off is lost so when the interception, which read the
/// body at its response stage, went on before it arrived (#1732).
fn finished_document(from_disk_cache: bool, encoded: f64) -> Vec<RawEvent> {
    vec![
        event(
            "Network.requestWillBeSent",
            json!({
                "requestId": DOCUMENT_REQUEST,
                "loaderId": LOADER,
                "documentURL": URL,
                "request": {
                    "url": URL,
                    "method": "GET",
                    "headers": {},
                    "initialPriority": "VeryHigh",
                    "referrerPolicy": "strict-origin-when-cross-origin",
                },
                "timestamp": 1.0,
                "wallTime": 1.0,
                "initiator": { "type": "other" },
                "redirectHasExtraInfo": false,
                "type": "Document",
                "frameId": FRAME,
            }),
        ),
        event(
            "Network.responseReceived",
            json!({
                "requestId": DOCUMENT_REQUEST,
                "loaderId": LOADER,
                "timestamp": 1.0,
                "type": "Document",
                "response": {
                    "url": URL,
                    "status": 200,
                    "statusText": "OK",
                    "headers": { "Transfer-Encoding": "chunked" },
                    "mimeType": "text/html",
                    "charset": "utf-8",
                    "connectionReused": false,
                    "connectionId": 1.0,
                    "encodedDataLength": 67.0,
                    "securityState": "neutral",
                    "fromDiskCache": from_disk_cache,
                },
                "hasExtraInfo": false,
                "frameId": FRAME,
            }),
        ),
        event(
            "Page.frameNavigated",
            json!({
                "frame": {
                    "id": FRAME,
                    "loaderId": LOADER,
                    "url": URL,
                    "domainAndRegistry": "site.test",
                    "securityOrigin": "https://site.test",
                    "mimeType": "text/html",
                    "secureContextType": "Secure",
                    "crossOriginIsolatedContextType": "NotIsolated",
                    "gatedAPIFeatures": [],
                },
                "type": "Navigation",
            }),
        ),
        event(
            "Network.dataReceived",
            json!({
                "requestId": DOCUMENT_REQUEST,
                "timestamp": 1.0,
                "dataLength": 205,
                "encodedDataLength": 0,
            }),
        ),
        event(
            "Network.loadingFinished",
            json!({
                "requestId": DOCUMENT_REQUEST,
                "timestamp": 1.0,
                "encodedDataLength": encoded,
            }),
        ),
        event(
            "Page.lifecycleEvent",
            json!({ "frameId": FRAME, "loaderId": LOADER, "name": "load", "timestamp": 1.0 }),
        ),
    ]
}

/// How the navigation of `LOADER` ended once the main frame's `events`
/// were read.
fn ended_after(events: &[RawEvent]) -> Option<Result<Arrived, Ended>> {
    let mut navigation = Navigation::of(FRAME.to_owned());
    for event in events {
        navigation.feed(event);
    }
    navigation.outcome(&Expected::Loader(LOADER.to_owned()), WaitUntil::Load, false)
}

/// A document from the network whose load the browser reported finished
/// with no bytes counted broke off, whatever its framing: the network
/// finishes a load it read whole only once it told the browser so, so its
/// failure was lost on the way (#1732). It raises `response_truncated`, as
/// the failure would have.
#[test]
fn a_document_from_the_network_finished_with_no_bytes_counted_is_truncated() {
    assert_eq!(
        ended_after(&finished_document(false, 0.0)),
        Some(Err(Ended::Truncated {
            received: 205,
            expected: None,
        }))
    );
}

/// A document the network counted, or the disk cache answered (which
/// counts no bytes, and never holds a cut body), arrived whole.
#[rstest::rstest]
#[case::counted_by_the_network(false, 494.0)]
#[case::from_the_disk_cache(true, 0.0)]
fn a_document_counted_or_from_the_cache_arrives_whole(
    #[case] from_disk_cache: bool,
    #[case] encoded: f64,
) {
    assert!(matches!(
        ended_after(&finished_document(from_disk_cache, encoded)),
        Some(Ok(Arrived::Document(Some(_))))
    ));
}
