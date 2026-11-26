//! The engine's index of its Sessions and of their pages, from ID to
//! handle (ADR 0146; spec #578's shape of 2026-10-16), and the pages each
//! Session holds itself. The index is written when a Session opens or
//! ends, and when a page is kept or goes; a Session's pages are its own,
//! so no lock holds every Session of the engine with what it holds. It
//! also keeps, until their Run ends, what a call on a Session a panic
//! closed raises once the Session is gone (ADR 0148).
//!
//! Neither lock is ever taken under the other, nor held across an
//! `.await`: a page is kept in its Session before the index names it, and
//! a Session's end takes it out of the index before its pages go, so a
//! page the index names is always found in its Session, or has just gone.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;

use super::{OpenPage, Session};
use crate::live_view::WatchedTabs;

/// The engine's Sessions and their pages, each by its ID, to the Session's
/// handle. Clones share them.
#[derive(Clone, Default)]
pub(super) struct Index {
    #[expect(
        clippy::disallowed_types,
        reason = "two maps read and changed for one operation, never across another lock or an `.await`; the Sessions table's lock may be held around it, and is never taken under it, so there is no lock order to test under loom (ADR 0060)"
    )]
    entries: Arc<std::sync::Mutex<Entries>>,
    /// Each open Session's tabs, as the Live view watches them, changed
    /// with the entries, once their lock is released.
    watched: WatchedTabs,
}

/// What the index holds.
#[derive(Default)]
struct Entries {
    sessions: BTreeMap<SessionId, Arc<Session>>,
    /// Each page, by its Session, which holds it.
    pages: BTreeMap<PageId, Arc<Session>>,
    /// What a call on a Session a panic of one of its owner's tasks closed
    /// raises, by its ID with its Run's, until its Run ends (ADR 0148).
    panicked: BTreeMap<SessionId, (RunId, CallError)>,
}

impl Index {
    /// The entries, for one operation.
    fn entries(&self) -> std::sync::MutexGuard<'_, Entries> {
        // A panic under the lock leaves two maps that each stay whole.
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The open Session `session`.
    pub(super) fn session(&self, session: &SessionId) -> Option<Arc<Session>> {
        self.entries().sessions.get(session).cloned()
    }

    /// Its open Sessions' tabs, as the Live view watches them.
    pub(super) fn watched(&self) -> WatchedTabs {
        self.watched.clone()
    }

    /// Names the Session `open` `session`.
    pub(super) fn opened(&self, session: &SessionId, open: Arc<Session>) {
        self.entries().sessions.insert(session.clone(), open);
        self.watched.session_opened(session);
    }

    /// A panic of one of the Session `session`'s owner's tasks closes it:
    /// a call that no longer finds it raises `error`, until its Run ends.
    pub(super) fn panicked(&self, session: &SessionId, open: &Session, error: CallError) {
        self.entries()
            .panicked
            .insert(session.clone(), (open.run.id().clone(), error));
    }

    /// The `engine_panicked` of the panic that closed the Session
    /// `session`, if one did and its Run has not ended.
    pub(super) fn panic_of(&self, session: &SessionId) -> Option<CallError> {
        self.entries()
            .panicked
            .get(session)
            .map(|(_, error)| error.clone())
    }

    /// The Sessions of the Run `run`.
    pub(super) fn sessions_of(&self, run: &RunId) -> Vec<(SessionId, Arc<Session>)> {
        self.entries()
            .sessions
            .iter()
            .filter(|(_, open)| open.run.id() == run)
            .map(|(session, open)| (session.clone(), Arc::clone(open)))
            .collect()
    }

    /// The Session `open`, named `session`, ends: the index no longer
    /// names it nor its pages.
    pub(super) fn ended(&self, session: &SessionId, open: &Session) {
        let removed = {
            let mut entries = self.entries();
            entries
                .pages
                .retain(|_, held| !std::ptr::eq(Arc::as_ptr(held), open));
            if entries
                .sessions
                .get(session)
                .is_some_and(|named| std::ptr::eq(Arc::as_ptr(named), open))
            {
                entries.sessions.remove(session)
            } else {
                None
            }
        };
        if removed.is_some() {
            self.watched.session_closed(session);
        }
        // Dropped outside the lock, should it be the Session's last handle.
        drop(removed);
    }

    /// Every Session of the Run `run` ends with it: the index no longer
    /// names them nor their pages. Answers them.
    pub(super) fn run_ended(&self, run: &RunId) -> Vec<(SessionId, Arc<Session>)> {
        let ended: Vec<(SessionId, Arc<Session>)> = {
            let mut entries = self.entries();
            entries.pages.retain(|_, held| held.run.id() != run);
            entries.panicked.retain(|_, (of, _)| of != run);
            let ended: Vec<SessionId> = entries
                .sessions
                .iter()
                .filter(|(_, open)| open.run.id() == run)
                .map(|(session, _)| session.clone())
                .collect();
            ended
                .into_iter()
                .filter_map(|session| {
                    let open = entries.sessions.remove(&session)?;
                    Some((session, open))
                })
                .collect()
        };
        for (session, _) in &ended {
            self.watched.session_closed(session);
        }
        ended
    }

    /// Names the page `page`, which `open`, named `session`, holds, while
    /// the index names `open`: whether it did.
    pub(super) fn kept(&self, page: &PageId, (session, open): (&SessionId, &Arc<Session>)) -> bool {
        let mut entries = self.entries();
        let named = entries
            .sessions
            .get(session)
            .is_some_and(|named| Arc::ptr_eq(named, open));
        if named {
            entries.pages.insert(page.clone(), Arc::clone(open));
        }
        drop(entries);
        if named && let Some(held) = open.pages.get(page) {
            self.watched
                .page_opened(session, page, Arc::new(held.driver.clone()));
        }
        named
    }

    /// The Session that holds the page `page`.
    fn holder(&self, page: &PageId) -> Option<Arc<Session>> {
        self.entries().pages.get(page).cloned()
    }

    /// The page `page`, from the Session that holds it.
    pub(super) fn page(&self, page: &PageId) -> Option<OpenPage> {
        self.holder(page)?.pages.get(page)
    }

    /// The pages of the Run `run`, by ID.
    pub(super) fn pages_of(&self, run: &RunId) -> Vec<(PageId, OpenPage)> {
        self.sessions_of(run)
            .into_iter()
            .flat_map(|(_, open)| open.pages.all())
            .collect()
    }

    /// The page `page` goes: its Session no longer holds it, and the index
    /// no longer names it. Answers it, when it was held.
    pub(super) fn remove_page(&self, page: &PageId) -> Option<OpenPage> {
        let holder = self.holder(page)?;
        let removed = holder.pages.remove(page);
        self.entries().pages.remove(page);
        if let Some(held) = &removed {
            self.watched.page_closed(&held.session, page);
        }
        removed
    }
}

/// The pages a Session holds, by ID, until it ends.
#[derive(Default)]
pub(super) struct HeldPages(
    #[expect(
        clippy::disallowed_types,
        reason = "one map read and changed for one operation, never across another lock or an `.await`; the Sessions table's lock may be held around it, and is never taken under it, so there is no lock order to test under loom (ADR 0060)"
    )]
    std::sync::Mutex<Held>,
);

#[derive(Default)]
struct Held {
    pages: BTreeMap<PageId, OpenPage>,
    /// Set once the Session ended: it takes in no page after.
    ended: bool,
}

impl HeldPages {
    /// The pages, for one operation.
    fn held(&self) -> std::sync::MutexGuard<'_, Held> {
        // A panic under the lock leaves a map that stays whole.
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether the Session has not ended yet.
    pub(super) fn open(&self) -> bool {
        !self.held().ended
    }

    /// Holds the page `page`, `opened`, unless the Session ended: whether
    /// it did.
    #[must_use]
    pub(super) fn hold(&self, page: PageId, opened: OpenPage) -> bool {
        let mut held = self.held();
        if held.ended {
            drop(held);
            // Dropped outside the lock, as a page that goes is.
            drop(opened);
            return false;
        }
        held.pages.insert(page, opened);
        true
    }

    /// The page `page`.
    pub(super) fn get(&self, page: &PageId) -> Option<OpenPage> {
        self.held().pages.get(page).cloned()
    }

    /// Every page, by ID.
    pub(super) fn all(&self) -> Vec<(PageId, OpenPage)> {
        self.held()
            .pages
            .iter()
            .map(|(page_id, page)| (page_id.clone(), page.clone()))
            .collect()
    }

    /// Every page's ID.
    pub(super) fn ids(&self) -> Vec<PageId> {
        self.held().pages.keys().cloned().collect()
    }

    /// The first page `found` finds, by ID.
    pub(super) fn find(&self, found: impl Fn(&OpenPage) -> bool) -> Option<(PageId, OpenPage)> {
        self.held()
            .pages
            .iter()
            .find(|(_, page)| found(page))
            .map(|(page_id, page)| (page_id.clone(), page.clone()))
    }

    /// The page `page` goes.
    fn remove(&self, page: &PageId) -> Option<OpenPage> {
        self.held().pages.remove(page)
    }

    /// The Session ends: it takes in no page after, and every page it held
    /// goes. Answers their IDs.
    pub(super) fn end(&self) -> Vec<PageId> {
        let pages = {
            let mut held = self.held();
            held.ended = true;
            std::mem::take(&mut held.pages)
        };
        // Dropped outside the lock: a page's admission and its place in its
        // browser go back to locks of their own.
        pages.into_keys().collect()
    }
}
