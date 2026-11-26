//! A gesture's watch against a browser played in process: the order in
//! which the browser's pipe delivers an event and a command's answer is
//! replayed to the connection's driver.

use std::time::Duration;

use goodfetch_cdp::SessionId;
use goodfetch_test_support::runtime;
use serde_json::json;

use super::GestureWatch;
use crate::driver::navigation::Navigation;
use crate::driver::page_reader::PageReader;
use crate::driver::replay::{self, FRAME, Played, SESSION};

/// Where the form the click submitted goes.
const ACTION: &str = "https://site.test/account";

/// How many times the order is replayed: the watch read the event first in
/// each, never by chance.
const REPLAYS: usize = 64;

/// Plays the browser: reads the first command the client wrote, then sends,
/// in one write, the main frame's request to navigate and that command's
/// answer, as a renderer that sent the request before it answered a round
/// trip into it after the input.
async fn request_then_answer(mut browser: Played) {
    let command = browser.command().await;
    let requested = replay::event(
        "Page.frameRequestedNavigation",
        json!({
            "frameId": FRAME,
            "reason": "formSubmissionPost",
            "url": ACTION,
            "disposition": "currentTab",
        }),
    );
    let answered = replay::answer(
        &command,
        json!({ "result": { "type": "object", "value": { "state": "landed" } } }),
    );
    assert!(
        browser.send(&[requested, answered]).await,
        "the client left"
    );
    browser.hold().await;
}

/// The navigation a click requested, as the watch read it once the round
/// trip after the click answered, in one replay of the browser's order.
async fn requested_once_answered() -> Option<String> {
    let ((connection, driver), browser) = replay::connected();
    let session = SessionId(SESSION.to_owned());
    let reader = PageReader::new(&connection, &session);
    let mut watch = GestureWatch {
        reader: reader.clone(),
        events: reader.events_on(&session),
        navigation: Navigation::of(FRAME.to_owned()),
        ended: false,
    };
    let (_, (), requested, ()) = tokio::join!(
        driver.run(),
        reader.reading(None),
        async {
            let answered = watch
                .during(connection.send_raw(
                    "Runtime.callFunctionOn",
                    Some(&session),
                    json!({}),
                    Duration::from_secs(30),
                ))
                .await;
            assert!(answered.is_ok(), "{answered:?}");
            drop(connection);
            watch.navigation.requested().map(str::to_owned)
        },
        request_then_answer(browser)
    );
    requested
}

/// An event the browser sent before the answer of the round trip a gesture
/// makes after its input is read before the gesture's wait goes on, as the
/// browser's pipe orders them: the main frame's request to navigate, which
/// a form's submission sends in the click's own task, is never left unread
/// behind the answer, which would let the click answer before the
/// navigation it started (#1766).
#[test]
fn an_event_sent_before_the_round_trip_answered_is_read_before_it() {
    runtime().block_on(async {
        let mut read = 0;
        for _ in 0..REPLAYS {
            if requested_once_answered().await.as_deref() == Some(ACTION) {
                read += 1;
            }
        }

        assert_eq!(read, REPLAYS, "replays in which the request was read");
    });
}
