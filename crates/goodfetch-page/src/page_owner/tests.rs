//! A Page's owner's answer to a panic of one of its tasks (ADR 0148).

use goodfetch_engine::CallError;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::tasks::{BlockingPolicy, OwnerKind, TaskOwner};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_test_support::{TestEnv, runtime};
use serde_json::{Value, json};

use super::{PageOwner, PageParts};
use crate::gesture_order::GestureOrder;
use crate::interception::Interception;
use crate::interception::tests::hold_page;
use crate::live_run::{LiveRun, Served, played_page};
use crate::page_events::tests::started;

/// The parts of the page `page_id`, as its Session holds them, its events
/// read for the test's Run from a page a played browser drives, handed to
/// `body`, which runs beside the engine and the page's connection.
async fn with_parts<T>(page_id: &PageId, body: impl AsyncFnOnce(PageParts) -> T) -> T {
    let env = TestEnv::new();
    let (live, mut engine): (LiveRun, Served) = LiveRun::start(&env).await;
    let (driver, played, mut connection) = played_page();
    let session = SessionId("session-1".to_owned());
    let interception = Interception::default();
    interception.open_session(&session);
    let parts = PageParts {
        events: started(page_id, &driver, &live),
        interception: hold_page(&interception, page_id, &session),
    };
    let ran = tokio::select! {
        biased;
        ran = body(parts) => ran,
        () = &mut engine => panic!("the engine ended before the test"),
        () = &mut connection => panic!("the page's connection ended before the test"),
    };
    drop(live);
    // The Run ends once the page's reads did, at the connection's end.
    drop((driver, played));
    tokio::join!(engine, connection);
    ran
}

/// `closed` for the page `page_id`, as its Session raises it.
fn page_closed(page_id: &PageId) -> CallError {
    CallError::new(
        &CLOSED,
        "the page is closed",
        &json!({ "page_id": page_id }),
    )
}

/// The code and details of `error`.
fn raised(error: &CallError) -> (&'static str, Option<Value>) {
    (error.code(), error.details().cloned().map(Value::Object))
}

/// A panic of a task of a Page's owner closes the Page alone: the owner
/// answers `engine_panicked`, ended `page` with the Page's ID, to the
/// Session's watch of the Page and to an ask that finds it gone, while its
/// Session's owner goes on.
#[test]
fn a_panic_of_one_of_its_tasks_closes_the_page_with_engine_panicked() {
    let page_id = PageId("page-1".to_owned());

    let (watched, asked, session_goes_on) =
        runtime().block_on(with_parts(&page_id, async |parts| {
            // A Session's owner is its Run's child, as the Run registry
            // starts it: a panic under a Run is told to no owner above it.
            let engine = TaskOwner::root(BlockingPolicy::Inline);
            let run = engine.child_for(OwnerKind::Run);
            let session = run.child();
            let owner = session.child();
            owner.spawn(async { panic!("a bug in one of the Page's tasks") });
            let page = PageOwner::serving(
                owner,
                &session,
                &tracing::Span::none(),
                (&page_id, parts, GestureOrder::new().0),
                page_closed(&page_id),
                &Metrics::default(),
            );
            let watched = page.panicked().await;
            let asked = page.gone().await;
            let session_goes_on = !session.has_panicked() && !session.token().is_cancelled();
            engine.stop().await;
            (watched, asked, session_goes_on)
        }));

    let engine_panicked = (
        "engine_panicked",
        Some(json!({ "ended": "page", "id": "page-1" })),
    );
    assert_eq!(
        (raised(&watched), raised(&asked), session_goes_on),
        (engine_panicked.clone(), engine_panicked, true)
    );
}

/// An ask whose owner ended with its Session's owner, no task of it
/// having panicked, raises the Page's `closed`.
#[test]
fn an_ask_whose_owner_ended_without_a_panic_raises_closed() {
    let page_id = PageId("page-1".to_owned());

    let asked = runtime().block_on(with_parts(&page_id, async |parts| {
        let session = TaskOwner::root(BlockingPolicy::Inline);
        let page = PageOwner::start(
            &session,
            &tracing::Span::none(),
            (&page_id, parts, GestureOrder::new().0),
            page_closed(&page_id),
            &Metrics::default(),
        );
        session.cancel();
        let asked = page.gone().await;
        session.stop().await;
        asked
    }));

    assert_eq!(asked.code(), "closed");
}
