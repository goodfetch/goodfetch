//! A gesture's watch against a browser played in process: the order in
//! which the browser's pipe delivers an event and a command's answer is
//! replayed to the connection's driver.

use std::time::Duration;

use goodfetch_cdp::{Connection, Framer, SessionId};
use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

use super::GestureWatch;
use crate::driver::navigation::Navigation;

/// The page's session.
const SESSION: &str = "page";

/// The page's main frame.
const FRAME: &str = "main";

/// Where the form the click submitted goes.
const ACTION: &str = "https://site.test/account";

/// How many times the order is replayed: the watch read the event first in
/// each, never by chance.
const REPLAYS: usize = 64;

/// One message as the browser's pipe frames it: JSON, then a NUL byte.
fn framed(message: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
    bytes.push(0);
    bytes
}

/// Plays the browser: reads the first command the client wrote, then sends,
/// in one write, the main frame's request to navigate and that command's
/// answer, as a renderer that sent the request before it answered a round
/// trip into it after the input.
async fn request_then_answer(mut browser: DuplexStream) {
    let mut framer = Framer::default();
    let command: Value = loop {
        if let Some(bytes) = framer
            .next_frame()
            .expect("invariant: the client's frames fit")
        {
            break serde_json::from_slice(&bytes).expect("invariant: the client writes JSON");
        }
        let mut buffer = vec![0; 4096];
        let count = browser
            .read(&mut buffer)
            .await
            .expect("invariant: the pipe reads");
        assert_ne!(count, 0, "the client closed its end before a command");
        framer.push(&buffer[..count]);
    };
    let mut bytes = framed(&json!({
        "method": "Page.frameRequestedNavigation",
        "sessionId": SESSION,
        "params": {
            "frameId": FRAME,
            "reason": "formSubmissionPost",
            "url": ACTION,
            "disposition": "currentTab",
        },
    }));
    bytes.extend(framed(&json!({
        "id": command["id"],
        "sessionId": SESSION,
        "result": { "result": { "type": "object", "value": { "state": "landed" } } },
    })));
    browser
        .write_all(&bytes)
        .await
        .expect("invariant: the pipe writes");
    // Held open until the client is done, as a browser keeps its pipe.
    let _ = browser.read(&mut [0; 1]).await;
}

/// The navigation a click requested, as the watch read it once the round
/// trip after the click answered, in one replay of the browser's order.
async fn requested_once_answered() -> Option<String> {
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(reader, writer);
    let session = SessionId(SESSION.to_owned());
    let mut watch = GestureWatch {
        events: connection.subscribe(Some(&session)),
        navigation: Navigation::of(FRAME.to_owned()),
        ended: false,
    };
    let (_, requested, ()) = tokio::join!(
        driver.run(),
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
