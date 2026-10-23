//! What a page's or a Session's end does to its calls (spec #594; ADR
//! 0113): a call in flight raises the code its object's end names, a later
//! call raises `closed`, or `page_crashed` again after a crash, and a
//! Session's states are emitted as `tracing` events.
#![expect(
    clippy::disallowed_types,
    reason = "the test's log writer holds its bytes behind a standard Mutex, outside any loom model"
)]

use std::future::pending;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use goodfetch_engine::CallError;
use goodfetch_protocol::connection::ClosedValue;
use goodfetch_protocol::page::{BrowserExit, PageId, SessionId};
use goodfetch_test_support::{runtime, subscriber_writing_to};
use rstest::rstest;
use serde_json::{Value, json};

use super::{Ending, PageLife, SessionLife};
use crate::errors::{network_error, renderer_crashed};
use crate::sessions::lifecycle::{
    Attributes, Owner, PageEffect, PageEvent, PageState, SessionEffect, SessionEvent, SessionState,
    Source,
};

/// A fresh Session a program opened in a launched browser.
const LAUNCHED: Attributes = Attributes {
    source: Source::Launched,
    owner: Owner::Program,
};

/// A browser killed by `SIGKILL`.
const KILLED: BrowserExit = BrowserExit {
    code: None,
    signal: Some(9),
};

fn page() -> PageLife {
    PageLife::new(PageId("page-1".to_owned()))
}

/// An error's code and details.
fn raised(error: &CallError) -> (&'static str, Value) {
    (
        error.code(),
        Value::Object(error.details().cloned().unwrap_or_default()),
    )
}

/// What a call in flight on `life` raises when `ending` reaches the page
/// under it, and what the next call raises.
fn in_flight_then_later(life: &PageLife, ending: &Ending) -> (CallError, CallError) {
    runtime().block_on(async {
        let call = life.call(PageEvent::Gesture, pending::<Result<(), CallError>>());
        let end = async {
            tokio::task::yield_now().await;
            life.end(ending)
        };
        let (answered, ended) = tokio::join!(call, end);
        ended.expect("the table takes the ending");
        let later = life.call(PageEvent::Read, async { Ok(()) }).await;
        (
            answered.expect_err("the call in flight fails"),
            later.expect_err("the later call fails"),
        )
    })
}

/// Covers: story:5-27, story:5-28
#[rstest]
#[case::closed_in_the_browser(
    Ending::Destroyed,
    ("call_cancelled", json!({ "by": "browser", "closed": "page" })),
    ("closed", json!({ "page_id": "page-1" }))
)]
#[case::its_session_closed_by_the_program(
    Ending::Closed(ClosedValue::Session),
    ("call_cancelled", json!({ "by": "program", "closed": "session" })),
    ("closed", json!({ "page_id": "page-1" }))
)]
#[case::its_browser_quit(
    Ending::BrowserQuit,
    ("call_cancelled", json!({ "by": "browser", "closed": "browser" })),
    ("closed", json!({ "page_id": "page-1" }))
)]
#[case::its_renderer_crashed(
    Ending::RendererCrash,
    ("page_crashed", json!({ "browser_exit": null })),
    ("page_crashed", json!({ "browser_exit": null }))
)]
#[case::its_browser_killed_by_a_signal(
    Ending::BrowserCrash(Some(KILLED)),
    ("page_crashed", json!({ "browser_exit": { "code": null, "signal": 9 } })),
    ("page_crashed", json!({ "browser_exit": { "code": null, "signal": 9 } }))
)]
fn a_page_that_ends_ends_its_call_in_flight_and_refuses_the_next(
    #[case] ending: Ending,
    #[case] in_flight: (&str, Value),
    #[case] later: (&str, Value),
) {
    let life = page();

    let (answered, refused) = in_flight_then_later(&life, &ending);

    assert_eq!((raised(&answered), raised(&refused)), (in_flight, later));
}

/// A call whose own command failed as its page went raises what ended the
/// page once that is known: a navigation the browser abandoned under a
/// renderer crash is `page_crashed`, not `navigation_failed`.
///
/// Covers: story:5-27, story:5-28
#[rstest]
#[case::a_command_to_a_page_closed_in_the_browser(
    renderer_crashed(),
    Ending::Destroyed,
    ("call_cancelled", json!({ "by": "browser", "closed": "page" }))
)]
#[case::a_navigation_abandoned_under_a_renderer_crash(
    network_error("http://catalog.test/", "net::ERR_ABORTED"),
    Ending::RendererCrash,
    ("page_crashed", json!({ "browser_exit": null }))
)]
fn a_call_that_failed_as_its_page_went_raises_what_ended_the_page(
    #[case] failure: CallError,
    #[case] ending: Ending,
    #[case] expected: (&str, Value),
) {
    let life = page();

    let answered = runtime().block_on(async {
        let call = life.call(PageEvent::Goto, async { Err::<(), _>(failure) });
        let end = async {
            tokio::task::yield_now().await;
            life.end(&ending)
        };
        tokio::join!(call, end).0
    });

    assert_eq!(raised(&answered.expect_err("the call fails")), expected);
}

/// A navigation's own failure stays its own when the page goes on.
///
/// Covers: story:5-27
#[test]
fn a_navigation_that_failed_on_a_live_page_raises_its_own_failure() {
    let life = page();

    let answered = runtime().block_on(life.call(PageEvent::Goto, async {
        Err::<(), _>(network_error(
            "http://nowhere.invalid/",
            "net::ERR_NAME_NOT_RESOLVED",
        ))
    }));

    assert_eq!(
        raised(&answered.expect_err("the call fails")).0,
        "navigation_failed"
    );
}

/// `close()` on a crashed page releases what is left and it stays
/// `Crashed`; a crash racing a close ends the page with nothing to count
/// (spec #594's P30 and P33, decisions 3 and 14).
///
/// Covers: story:5-28
#[rstest]
#[case::a_close_after_a_crash(
    Ending::RendererCrash,
    Ending::Closed(ClosedValue::Page),
    (PageEffect::Release, PageState::Crashed)
)]
#[case::a_crash_racing_a_close(
    Ending::Closed(ClosedValue::Page),
    Ending::BrowserCrash(Some(KILLED)),
    (PageEffect::Nothing, PageState::Closed)
)]
fn a_second_ending_of_a_page_does_what_its_table_says(
    #[case] first: Ending,
    #[case] second: Ending,
    #[case] expected: (PageEffect, PageState),
) {
    let life = page();
    life.end(&first).expect("the first ending is allowed");

    let effect = life.end(&second).expect("the second ending is allowed");

    assert_eq!((effect, life.state()), expected);
}

/// A call on a Session still opening waits for the open, then runs, or
/// raises the open's code when it failed (spec #594's S20, decision 1).
///
/// Covers: story:5-28
#[rstest]
#[case::the_open_finishes(None, Ok(()))]
#[case::the_open_fails(Some(renderer_crashed()), Err("page_crashed"))]
fn a_call_on_an_opening_session_waits_for_its_open(
    #[case] failure: Option<CallError>,
    #[case] expected: Result<(), &str>,
) {
    let life = SessionLife::new(
        SessionId("run.session-1".to_owned()),
        LAUNCHED,
        SessionState::Opening,
    );

    let answered = runtime().block_on(async {
        let call = life.call();
        let open = async {
            tokio::task::yield_now().await;
            match failure {
                None => life.apply(SessionEvent::SourceReady, None),
                Some(error) => life.open_failed(error),
            }
        };
        tokio::join!(call, open).0
    });

    assert_eq!(answered.map_err(|error| error.code()), expected);
}

/// A Session whose browser crashed raises `page_crashed` with how the
/// browser exited on every later call, and `close()` releases what is left
/// with the state staying `Crashed` (spec #594's S9, S21 and S22).
///
/// Covers: story:5-28
#[test]
fn a_crashed_session_refuses_its_calls_and_stays_crashed_once_closed() {
    let life = SessionLife::new(
        SessionId("run.session-1".to_owned()),
        LAUNCHED,
        SessionState::Open,
    );
    life.apply(SessionEvent::BrowserCrash, Some(KILLED))
        .expect("an open Session takes its browser's crash");

    let refused = runtime().block_on(life.call());
    let closed = life.apply(SessionEvent::Close, None);

    assert_eq!(
        (
            raised(&refused.expect_err("the call is refused")),
            closed.map_err(|error| error.code()),
            life.state()
        ),
        (
            (
                "page_crashed",
                json!({ "browser_exit": { "code": null, "signal": 9 } })
            ),
            Ok(SessionEffect::Release),
            SessionState::Crashed
        )
    );
}

/// An event only a bug sends raises `internal` naming the object, the
/// state and the event (ADRs 0079 and 0128).
///
/// Covers: story:5-28
#[test]
fn an_event_a_state_cannot_receive_raises_internal_with_the_object_state_and_event() {
    let life = SessionLife::new(
        SessionId("run.session-1".to_owned()),
        LAUNCHED,
        SessionState::Open,
    );

    let refused = life.apply(SessionEvent::ChildrenClosed, None);

    assert_eq!(
        raised(&refused.expect_err("the event is refused")),
        (
            "internal",
            json!({ "object": "session", "state": "Open", "event": "ChildrenClosed" })
        )
    );
}

/// Bytes a test's subscriber writes.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Each change of a Session's state is a `tracing` event the Simulation's
/// scenarios read (ADR 0060).
///
/// Covers: story:5-28
#[test]
fn a_sessions_change_of_state_is_a_tracing_event() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = subscriber_writing_to(|| SystemTime::UNIX_EPOCH, move || writer.clone());
    let life = SessionLife::new(
        SessionId("run.session-1".to_owned()),
        LAUNCHED,
        SessionState::Open,
    );

    tracing::subscriber::with_default(subscriber, || {
        life.apply(SessionEvent::BrowserCrash, Some(KILLED))
            .expect("an open Session takes its browser's crash");
    });

    let written = String::from_utf8(
        captured
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    )
    .expect("the log is UTF-8");
    let change = written
        .lines()
        .find(|line| line.contains("a Session changed state"))
        .unwrap_or_else(|| panic!("no state change in {written}"));
    assert!(
        [
            "goodfetch::session",
            "run.session-1",
            "Open",
            "Crashed",
            "BrowserCrash"
        ]
        .iter()
        .all(|field| change.contains(field)),
        "{change}"
    );
}
