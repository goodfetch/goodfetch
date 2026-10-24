//! The interception's tables, without a browser: what a page's or a
//! Session's end takes out of them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_engine::egress::Policy;
use goodfetch_protocol::page::{PageId, SessionId};

use super::{EngineRequests, Interception, Learned, PageState, lock, steps};
use crate::page_value::Permit;

/// Puts the page `page` of the Session `session` in `interception`'s
/// tables, as its start does once the browser paused its requests.
fn hold_page(interception: &Interception, page: &PageId, session: &SessionId) -> Arc<PageState> {
    let mut tables = lock(&interception.shared.tables);
    let pending = Arc::clone(tables.pending.entry(session.clone()).or_default());
    let routes = Arc::clone(tables.sessions.entry(session.clone()).or_default());
    let state = Arc::new(PageState {
        routes: Arc::default(),
        session: routes,
        failure: Mutex::new(None),
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
    });
    tables.pages.insert(page.clone(), Arc::clone(&state));
    state
}

/// A page that closes before its navigation's `load` gives back the permit
/// a service worker's request took for that navigation, so a pooled
/// Session's domain is not held for its life.
#[test]
fn a_page_that_closes_before_load_gives_back_its_navigations_permit() {
    let interception = Interception::default();
    let (page, session) = (
        PageId("page-1".to_owned()),
        SessionId("session-1".to_owned()),
    );
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
