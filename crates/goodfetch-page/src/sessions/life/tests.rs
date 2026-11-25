//! What a page's or a Session's end does to its calls (spec #594; ADR
//! 0113): a call in flight raises the code its object's end names, a later
//! call raises `closed`, or `page_crashed` again after a crash, and a
//! Session's states are emitted as `tracing` events.

use std::future::pending;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::metrics::Metrics;
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CallCancelledDetails, CancelledBy, ClosedValue,
};
use goodfetch_protocol::governor::BROWSER_PAGE_MEMORY_CAP;
use goodfetch_protocol::page::{BrowserExit, PageId, SessionId};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use goodfetch_test_support::{CapturedLog, runtime, subscriber_writing_to};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::{Ending, PageLife, SessionLife};
use crate::errors::{network_error, page_panicked, renderer_crashed};
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
    PageLife::new(PageId("page-1".to_owned()), None, no_routes())
}

/// How many of a page's requests wait on a route handler, for a page none
/// of whose requests ever does.
fn no_routes() -> watch::Receiver<usize> {
    watch::channel(0).1
}

/// What ran, in order, on a page with a navigation pending when a second
/// call arrived, `routed` of its requests waiting on a route handler: the
/// second call's work, the navigation's end, or both in the order they
/// ran.
fn behind_a_pending_navigation(second: PageEvent, routed: usize) -> Vec<&'static str> {
    let (held, routed_now) = watch::channel(routed);
    let life = PageLife::new(PageId("page-1".to_owned()), None, routed_now);
    let ran = std::cell::RefCell::new(Vec::new());
    runtime().block_on(async {
        let (navigated, navigation) = tokio::sync::oneshot::channel::<()>();
        let navigating = life.call(PageEvent::Goto, async {
            let _ = navigation.await;
            ran.borrow_mut().push("navigation");
            Ok(())
        });
        let other = life.call(second, async {
            ran.borrow_mut().push("second call");
            Ok(())
        });
        let end = async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
            let _ = navigated.send(());
        };
        let _ = tokio::join!(navigating, other, end);
    });
    drop(held);
    ran.into_inner()
}

/// A navigation call holds the page's gesture order (spec #594's P14,
/// ADR 0085): a gesture sent while it is pending runs once it ended,
/// unless one of the page's requests waits on a route handler, when the
/// navigation gives up its turn; a read runs meanwhile, and a second
/// navigation runs at once, interrupting the pending one (P37).
///
/// Covers: story:5-27
#[rstest]
#[case::a_gesture(PageEvent::Gesture, 0, vec!["navigation", "second call"])]
#[case::a_gesture_while_a_route_handler_holds_a_request(PageEvent::Gesture, 1, vec!["second call", "navigation"])]
#[case::a_read(PageEvent::Read, 0, vec!["second call", "navigation"])]
#[case::a_second_navigation(PageEvent::Goto, 0, vec!["second call", "navigation"])]
fn a_pending_navigation_holds_the_gesture_order(
    #[case] second: PageEvent,
    #[case] routed: usize,
    #[case] expected: Vec<&'static str>,
) {
    assert_eq!(behind_a_pending_navigation(second, routed), expected);
}

/// A navigation that ended leaves its page `Open`, and one a second
/// navigation interrupted leaves it `Navigating` until that one ends.
///
/// Covers: story:5-27
#[test]
fn a_page_is_navigating_while_its_latest_navigation_is_pending() {
    let life = page();

    let states = runtime().block_on(async {
        let (first_ended, first) = tokio::sync::oneshot::channel::<()>();
        let (second_ended, second) = tokio::sync::oneshot::channel::<()>();
        let first = life.call(PageEvent::Goto, async {
            let _ = first.await;
            Ok(())
        });
        let second = life.call(PageEvent::Goto, async {
            let _ = second.await;
            Ok(())
        });
        let observe = async {
            tokio::task::yield_now().await;
            let pending = state_of(&life);
            let _ = first_ended.send(());
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            let interrupted = state_of(&life);
            let _ = second_ended.send(());
            (pending, interrupted)
        };
        let (_, _, states) = tokio::join!(first, second, observe);
        states
    });

    assert_eq!(
        (states, state_of(&life)),
        (
            (PageState::Navigating, PageState::Navigating),
            PageState::Open
        )
    );
}

/// Where `life`'s page is, as its state watch holds it.
fn state_of(life: &PageLife) -> PageState {
    *life.shared.state.borrow()
}

/// A lease check whose lease ended, as a crawl handler's past its work
/// item's lease.
#[derive(Debug)]
struct Ended;

impl LeaseCheck for Ended {
    fn check(&self) -> Result<(), LeaseEnded> {
        Err(LeaseEnded::Lost)
    }

    /// `call_cancelled`, by the program, in place of the Product's own
    /// code.
    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        CallError::new(
            &CALL_CANCELLED,
            format!("the lease ended ({ended}): `{method}` withheld"),
            &CallCancelledDetails {
                by: CancelledBy::Program,
                closed: None,
                person: None,
            },
        )
    }
}

/// What the lease check `Ended` raises.
fn withheld() -> CallError {
    Ended.withheld(LeaseEnded::Lost, "Input.dispatchMouseEvent")
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
#[case::its_lease_ended(
    Ending::LeaseEnded(withheld()),
    ("call_cancelled", json!({ "by": "program" })),
    ("closed", json!({ "page_id": "page-1" }))
)]
#[case::a_panic_of_its_owners_task(
    Ending::Panicked(page_panicked(&Metrics::default(), &PageId("page-1".to_owned()), None)),
    ("engine_panicked", json!({ "ended": "page", "id": "page-1" })),
    ("engine_panicked", json!({ "ended": "page", "id": "page-1" }))
)]
#[case::its_reader_fell_behind(
    Ending::EventsLost,
    ("page_events_lost", json!({ "page_id": "page-1" })),
    ("page_events_lost", json!({ "page_id": "page-1" }))
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

/// A page's target that went after the program asked the page to close,
/// running its `beforeunload` handlers, closed as the program asked: its
/// call in flight raises `call_cancelled` naming the program, and its end
/// confirms that close (spec #594's P8 and P29). Unasked, the target's end
/// is a close in the browser (P9).
///
/// Covers: story:5-27
#[rstest]
#[case::asked_by_the_program(
    true,
    ("call_cancelled", json!({ "by": "program", "closed": "page" })),
    PageEffect::Confirm
)]
#[case::unasked(
    false,
    ("call_cancelled", json!({ "by": "browser", "closed": "page" })),
    PageEffect::ClosedInBrowser
)]
fn a_target_that_went_after_the_program_asked_its_page_to_close_ends_as_its_close(
    #[case] asked: bool,
    #[case] in_flight: (&str, Value),
    #[case] effect: PageEffect,
) {
    let life = page();
    if asked {
        life.ask_to_close();
    }

    let (answered, ended) = runtime().block_on(async {
        let call = life.call(PageEvent::Gesture, pending::<Result<(), CallError>>());
        let end = async {
            tokio::task::yield_now().await;
            life.destroyed()
        };
        tokio::join!(call, end)
    });

    assert_eq!(
        (
            raised(&answered.expect_err("the call in flight fails")),
            ended.map_err(|error| error.code()),
            state_of(&life),
        ),
        (in_flight, Ok(effect), PageState::Closed)
    );
}

/// A call whose command its lease check withheld raises the lease check's
/// error and moves its page to `Closing`, its next call raising `closed`
/// (spec #594's P31, decision 13): the page then closes.
///
/// Covers: story:5-135
#[test]
fn a_call_its_lease_withheld_moves_its_page_to_closing() {
    let life = PageLife::new(
        PageId("page-1".to_owned()),
        Some(Arc::new(Ended)),
        no_routes(),
    );

    let (answered, later, closing) = runtime().block_on(async {
        let answered = life
            .call(PageEvent::Gesture, async { Err::<(), _>(withheld()) })
            .await;
        let later = life.call(PageEvent::Read, async { Ok(()) }).await;
        let closing = tokio::time::timeout(Duration::from_secs(1), life.ended_by_lease()).await;
        (answered, later, closing.is_ok())
    });

    assert_eq!(
        (
            raised(&answered.expect_err("the call is withheld")),
            raised(&later.expect_err("the later call is refused")),
            state_of(&life),
            closing,
        ),
        (
            ("call_cancelled", json!({ "by": "program" })),
            ("closed", json!({ "page_id": "page-1" })),
            PageState::Closing,
            true,
        )
    );
}

/// A call that failed on its own while its lease holds leaves its page
/// open.
///
/// Covers: story:5-27
#[test]
fn a_failed_call_under_a_lease_that_holds_leaves_its_page_open() {
    let life = PageLife::new(
        PageId("page-1".to_owned()),
        Some(Arc::new(Holds)),
        no_routes(),
    );

    let answered = runtime().block_on(life.call(PageEvent::Gesture, async {
        Err::<(), _>(renderer_crashed())
    }));

    assert_eq!(
        (answered.map_err(|error| error.code()), state_of(&life)),
        (Err("page_crashed"), PageState::Open)
    );
}

/// A lease check whose lease holds.
#[derive(Debug)]
struct Holds;

impl LeaseCheck for Holds {
    fn check(&self) -> Result<(), LeaseEnded> {
        Ok(())
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        Ended.withheld(ended, method)
    }
}

/// A call whose own command failed as its page went raises what ended the
/// page once that is known. (A navigation the browser abandoned under a
/// renderer crash is told apart by the driver, which raises `page_crashed`
/// itself.)
///
/// Covers: story:5-27, story:5-28
#[rstest]
#[case::a_command_to_a_page_closed_in_the_browser(
    renderer_crashed(),
    Ending::Destroyed,
    ("call_cancelled", json!({ "by": "browser", "closed": "page" }))
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
    (PageEffect::CloseOvertaken, PageState::Closed)
)]
#[case::a_quit_racing_a_close(
    Ending::Closed(ClosedValue::Page),
    Ending::BrowserQuit,
    (PageEffect::CloseOvertaken, PageState::Closed)
)]
fn a_second_ending_of_a_page_does_what_its_table_says(
    #[case] first: Ending,
    #[case] second: Ending,
    #[case] expected: (PageEffect, PageState),
) {
    let life = page();
    life.end(&first).expect("the first ending is allowed");

    let effect = life.end(&second).expect("the second ending is allowed");

    assert_eq!((effect, state_of(&life)), expected);
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

/// `close()` on a Session still opening abandons the open: a call that
/// waited on it raises `call_cancelled` naming the program and the
/// Session, and a call made once it is closing raises `closed` (spec
/// #594's S3 and R).
///
/// Covers: story:5-28
#[test]
fn a_call_waiting_on_an_abandoned_open_raises_call_cancelled_and_a_later_one_closed() {
    let life = SessionLife::new(
        SessionId("run.session-1".to_owned()),
        LAUNCHED,
        SessionState::Opening,
    );

    let (waited, closed, later) = runtime().block_on(async {
        let call = life.call();
        let close = async {
            tokio::task::yield_now().await;
            life.apply(SessionEvent::Close, None)
        };
        let (waited, closed) = tokio::join!(call, close);
        (waited, closed, life.call().await)
    });

    assert_eq!(
        (
            raised(&waited.expect_err("the waiting call is cancelled")),
            closed.map_err(|error| error.code()),
            raised(&later.expect_err("the later call is refused")),
        ),
        (
            (
                "call_cancelled",
                json!({ "by": "program", "closed": "session" })
            ),
            Ok(SessionEffect::AbandonOpen),
            ("closed", json!({ "session_id": "run.session-1" })),
        )
    );
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
/// Covers: story:5-28, error:internal
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

/// A fresh Session a crawl's Session pool opened.
const POOLS: Attributes = Attributes {
    source: Source::Launched,
    owner: Owner::Pool,
};

/// A fresh Session an Agent opened.
const AGENTS: Attributes = Attributes {
    source: Source::Launched,
    owner: Owner::Agent,
};

/// The Session pool's `Retire` closes a pool's Session's pages and
/// context; on a Session a program or an Agent opened it is a bug,
/// `internal` (those meet `blocked` instead, ADR 0088); and a `Retire`
/// racing a `Close`, or after it, changes nothing (spec #594's S11 and
/// L).
///
/// Covers: story:5-138
#[rstest]
#[case::a_pools_session(POOLS, &[], Ok((SessionState::Closing, SessionEffect::CloseChildren)))]
#[case::racing_a_close(POOLS, &[SessionEvent::Close], Ok((SessionState::Closing, SessionEffect::Nothing)))]
#[case::after_a_close(
    POOLS,
    &[SessionEvent::Close, SessionEvent::ChildrenClosed],
    Ok((SessionState::Closed, SessionEffect::Nothing))
)]
#[case::a_programs_session(LAUNCHED, &[], Err("internal"))]
#[case::an_agents_session(AGENTS, &[], Err("internal"))]
fn the_pool_retires_only_its_own_sessions_and_a_retire_racing_a_close_changes_nothing(
    #[case] attributes: Attributes,
    #[case] before: &[SessionEvent],
    #[case] expected: Result<(SessionState, SessionEffect), &str>,
) {
    let life = SessionLife::new(
        SessionId("run.session-1".to_owned()),
        attributes,
        SessionState::Open,
    );
    for event in before {
        life.apply(*event, None).expect("the table takes the event");
    }

    let retired = life.apply(SessionEvent::Retire, None);

    assert_eq!(
        retired
            .map(|effect| (life.state(), effect))
            .map_err(|error| error.code()),
        expected
    );
}

/// Each change of a Session's state is a `tracing` event the Simulation's
/// scenarios read (ADR 0060).
///
/// Covers: story:5-28
#[test]
fn a_sessions_change_of_state_is_a_tracing_event() {
    let captured = CapturedLog::default();
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

    let written = String::from_utf8(captured.written()).expect("the log is UTF-8");
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

/// The Resource governor's end of a page past its memory cap.
fn past_cap() -> Ending {
    Ending::MemoryExceeded {
        memory: 64 << 20,
        limit: SettingLimit {
            key: BROWSER_PAGE_MEMORY_CAP.name.to_owned(),
            value: json!(32 << 20),
            layer: SettingsLayer::Run,
            origin: None,
        },
    }
}

/// A page the Resource governor ended past its memory cap stays ended past
/// it whatever ends its close, so its later calls raise
/// `page_memory_exceeded` again once it left; a page that ended otherwise
/// first, the cap found later, does not.
///
/// Covers: story:3-59
#[rstest]
#[case::its_close_confirmed(vec![past_cap(), Ending::Destroyed], true)]
#[case::its_close_overtaken_by_a_crash(
    vec![past_cap(), Ending::BrowserCrash(Some(KILLED)), Ending::Destroyed],
    true
)]
#[case::its_close_overtaken_by_a_quit(vec![past_cap(), Ending::BrowserQuit], true)]
#[case::closed_again_by_its_session(
    vec![past_cap(), Ending::Closed(ClosedValue::Session), Ending::Destroyed],
    true
)]
#[case::closed_by_the_program_first(
    vec![Ending::Closed(ClosedValue::Page), past_cap(), Ending::Destroyed],
    false
)]
#[case::its_lease_ended_first(
    vec![Ending::LeaseEnded(withheld()), past_cap(), Ending::Destroyed],
    false
)]
#[case::its_renderer_crashed_first(vec![Ending::RendererCrash, past_cap()], false)]
fn a_page_ended_past_its_memory_cap_stays_so_whatever_ends_its_close(
    #[case] endings: Vec<Ending>,
    #[case] past: bool,
) {
    let life = page();

    for ending in &endings {
        life.end(ending).expect("each end is one the table takes");
    }

    assert_eq!(life.keeps_its_end(), past);
}
