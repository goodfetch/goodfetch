//! The interception's tables, without a browser: what a page's or a
//! Session's end takes out of them, and where a handler's failure waits.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_engine::CallError;
use goodfetch_engine::egress::Policy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;

use super::{EngineRequests, Interception, Learned, LeftFailure, PageState, lock, steps};
use crate::page_value::Permit;

/// The interception state of a page or a service worker of the Session
/// `session` in `interception`'s tables: its handlers' failures wait in
/// `failure`, the page's own slot, or its Session's for a worker.
fn state_of(
    interception: &Interception,
    session: &SessionId,
    failure: Option<LeftFailure>,
) -> PageState {
    let mut tables = lock(&interception.shared.tables);
    let pending = Arc::clone(tables.pending.entry(session.clone()).or_default());
    let routes = Arc::clone(tables.sessions.entry(session.clone()).or_default());
    let session_failure = Arc::clone(tables.failures.entry(session.clone()).or_default());
    PageState {
        routes: Arc::default(),
        session: routes,
        failure: failure.unwrap_or_else(|| Arc::clone(&session_failure)),
        session_failure,
        cosmetic: None,
        engine_requests: EngineRequests {
            policy: Policy::new(&["*"], &[] as &[&str]).expect("invariant: `*` is a host pattern"),
            blocking: steps::Blocking {
                never_block: Arc::new(NeverBlock::new()),
                lists: None,
                choices: goodfetch_clean_up_rules::matching::Choices::default(),
            },
        },
        pending,
        navigating: None,
        files: Mutex::new(None),
        lists_on: false,
        retrying: AtomicBool::new(false),
        lists_blocked: std::sync::atomic::AtomicU64::new(0),
        learned: Arc::new(Learned::default()),
        on_document: None,
    }
}

/// Puts the page `page` of the Session `session` in `interception`'s
/// tables, as its start does once the browser paused its requests.
fn hold_page(interception: &Interception, page: &PageId, session: &SessionId) -> Arc<PageState> {
    let state = Arc::new(state_of(
        interception,
        session,
        Some(LeftFailure::default()),
    ));
    lock(&interception.shared.tables)
        .pages
        .insert(page.clone(), Arc::clone(&state));
    state
}

/// The page and Session the tests hold.
fn page_and_session() -> (PageId, SessionId) {
    (
        PageId("page-1".to_owned()),
        SessionId("session-1".to_owned()),
    )
}

/// A page that closes before its navigation's `load` gives back the permit
/// a service worker's request took for that navigation, so a pooled
/// Session's domain is not held for its life.
#[test]
fn a_page_that_closes_before_load_gives_back_its_navigations_permit() {
    let interception = Interception::default();
    let (page, session) = page_and_session();
    interception.open_session(&session);
    let state = hold_page(&interception, &page, &session);
    state
        .pending
        .started(&page, "http://a.test/#top", &Arc::downgrade(&state));
    let carried = state.pending.take("http://a.test/");
    let released = Arc::new(AtomicBool::new(false));
    let releasing = Arc::clone(&released);
    state.pending.hold(
        &page,
        Permit::new(move || releasing.store(true, Ordering::SeqCst)),
    );

    interception.close_pages(std::slice::from_ref(&page));

    assert_eq!(
        (
            carried.map(|(page, _)| page),
            released.load(Ordering::SeqCst)
        ),
        (Some(page), true)
    );
}

/// A Session route's handler that fails on a service worker's request,
/// which belongs to no page, leaves its failure on the Session: the next
/// call on a page of the Session raises it, once.
#[test]
fn a_failure_on_a_service_workers_request_waits_for_the_sessions_next_call() {
    let interception = Interception::default();
    let (page, session) = page_and_session();
    interception.open_session(&session);
    hold_page(&interception, &page, &session);
    let worker = state_of(&interception, &session, None);

    worker.leave(CallError::new(
        &CLOSED,
        "the handler failed",
        &serde_json::json!({}),
    ));

    assert_eq!(
        (
            interception
                .take_failure(&page)
                .map(|failure| failure.code()),
            interception.take_failure(&page).is_none(),
        ),
        (Some(CLOSED.name), true)
    );
}

/// A Run's pages and served documents read with one set of parsed
/// selectors while any of them holds it, so the Run parses each selector
/// once (#1765); another Run has its own, and the interception keeps none
/// once the Run's last holder lets go of it.
#[test]
fn a_runs_documents_share_their_parsed_selectors_while_any_holds_them() {
    let interception = Interception::default();
    let [one, other] = ["run-1", "run-2"].map(|run| RunId::parse(run).unwrap());

    let work = goodfetch_counted_work::CountedWork::default();
    let first = interception.shared.selectors(&one, &work);
    let [again, others] = [&one, &other].map(|run| interception.shared.selectors(run, &work));
    let shared = [Arc::ptr_eq(&first, &again), Arc::ptr_eq(&first, &others)];
    let held = Arc::downgrade(&first);
    drop((first, again));

    assert_eq!((shared, held.upgrade().is_none()), ([true, false], true));
}
