//! A Page's one reader (spec #2256, "One reader per Page"): one
//! subscription of the browser's connection, with one bounded buffer,
//! listed under the page's session and under the session of each
//! out-of-process frame and worker one of its parts reads, whose events one
//! task reads and hands on to each part subscribed to it, by session, as the
//! connection hands its own. A part subscribes to its page's reader, never to
//! the connection, so an event of a page's sessions visits one reader in the
//! connection's table, however many parts read it (ADR 0140's
//! `C1.event_visits`).
//!
//! A part's subscription takes the events the connection took in after it
//! subscribed, though the reader hands them on later, and ends as one of the
//! connection's would: its session's target detached, which the reader reads
//! on the page's or a frame's session for a frame or worker and the
//! connection tells it for the page, its holder fell behind, or the
//! connection ended. A reader that fell behind ends every part's
//! subscription, saying so, as each one's would have.
//!
//! The connection hands an event to its subscribers before it hands over a
//! later answer, which a part that reads its events while a command runs,
//! a navigation's or a Gesture's, counts on (#1766, #1770): the reader
//! answers an ask only once it holds no event unread, so a part that takes
//! a command's answer once its reader answered
//! ([`PageReader::after_events`]) finds in its own subscription every event
//! the browser sent before that answer. The reader is also its page's
//! origins watcher (`carry`): it adds the origin of each document its
//! page's session shows, and answers a capture's ask the same way, so the
//! capture lists every origin the browser reported before an answer the
//! capture already holds.
#![expect(
    clippy::disallowed_types,
    reason = "the sessions a reader is listed under are changed one at a time, the connection's joining list locked inside for the one listing, which never takes this lock, and never across an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::generated::target;
use goodfetch_cdp::{
    Connection, Handing, Handout, MoreSessions, SessionId as TargetSession, Subscription,
    SubscriptionEnd,
};
use tokio::sync::{mpsc, oneshot};

use super::carry::Watcher;

/// How many asks a reader holds unread: a page's parts ask one at a time
/// each, a few at once.
const ASKS: usize = 16;

/// An ask of a reader, answered once it holds no event unread.
type Ask = oneshot::Sender<()>;

/// What a reader's task takes in once: its subscription, its handing and
/// its asks.
type Reading = (Subscription, Handing, mpsc::Receiver<Ask>);

/// A page's one reader, which its drivers and the drivers of its children
/// share.
#[derive(Clone, Debug)]
pub(crate) struct PageReader(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    /// The page's own session, which its subscription was made for.
    page: TargetSession,
    /// Where its parts subscribe.
    handout: Handout,
    /// What lists its subscription under a child's session.
    more: MoreSessions,
    /// The children's sessions its subscription is listed under.
    listed: Mutex<BTreeSet<TargetSession>>,
    /// How a part asks it whether it handed on every event the connection
    /// took in so far.
    asks: mpsc::Sender<Ask>,
    /// Its subscription, its handing and its asks, until its task takes
    /// them.
    reading: Mutex<Option<Reading>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PageReader {
    /// The reader of the page on `page`, subscribed to `connection` from
    /// now on, whose events wait for its task ([`Self::reading`]).
    pub(crate) fn new(connection: &Connection, page: &TargetSession) -> Self {
        let events = connection.subscribe(Some(page));
        let (handout, handing) = connection.handout();
        let (asks, asked) = mpsc::channel(ASKS);
        Self(Arc::new(Shared {
            page: page.clone(),
            handout,
            more: events.more_sessions(),
            listed: Mutex::default(),
            asks,
            reading: Mutex::new(Some((events, handing, asked))),
        }))
    }

    /// The events of `session`, the page's or one of its children's, that
    /// the connection takes in from now on, which the reader hands on; the
    /// reader is listed under a child's session the first time a part asks.
    pub(crate) fn events_on(&self, session: &TargetSession) -> Subscription {
        if *session != self.0.page {
            let mut listed = lock(&self.0.listed);
            if listed.insert(session.clone()) {
                self.0.more.add(Some(session));
            }
        }
        self.0.handout.subscribe(Some(session))
    }

    /// Waits until the reader handed on every event the connection took in
    /// before this was asked; at once once it ended. Its task must have
    /// been spawned ([`Self::reading`]).
    pub(crate) async fn caught_up(&self) {
        let (answer, answered) = oneshot::channel();
        if self.0.asks.send(answer).await.is_ok() {
            let _ = answered.await;
        }
    }

    /// The answer of `command`, taken once the reader handed on every event
    /// the connection took in before it: a part that reads its events first
    /// whenever both are ready, as a navigation and a Gesture do, has read
    /// each of them once this answers.
    pub(crate) async fn after_events<T>(&self, command: impl Future<Output = T>) -> T {
        let answer = command.await;
        self.caught_up().await;
        answer
    }

    /// The reader's task: it reads the page's events until its subscription
    /// ends, adding each origin to `origins` when given and answering asks,
    /// then ends every part's subscription as its own ended. Only the first
    /// call reads; a later one ends at once.
    pub(crate) fn reading(&self, origins: Option<Watcher>) -> impl Future<Output = ()> + use<> {
        let taken = lock(&self.0.reading).take();
        let shared = Arc::clone(&self.0);
        async move {
            if let Some((events, handing, asked)) = taken {
                read(&shared, events, handing, asked, origins).await;
            }
        }
    }
}

/// Reads `events`, handing each on with `handing`, until they end: a
/// child's detach ends its parts' subscriptions first, and `origins`, when
/// given, takes each event of the page's session. An ask, `asked`'s or
/// `origins`', is answered only when no event is left unread.
async fn read(
    shared: &Shared,
    mut events: Subscription,
    mut handing: Handing,
    mut asked: mpsc::Receiver<Ask>,
    mut origins: Option<Watcher>,
) {
    loop {
        let event = tokio::select! {
            biased;
            event = events.next() => event,
            Some(answer) = asked.recv() => {
                let _ = answer.send(());
                continue;
            }
            () = watcher_asked(origins.as_mut()) => continue,
        };
        let Some(event) = event else {
            break;
        };
        if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
            let gone = TargetSession(detached.session_id);
            lock(&shared.listed).remove(&gone);
            handing.detached(&gone);
        }
        handing.hand(&event);
        if let Some(watcher) = &origins
            && event.session() == Some(&shared.page)
        {
            watcher.read(&event);
        }
    }
    handing.end(events.ended().unwrap_or(SubscriptionEnd::Disconnected));
}

/// A capture's ask of `origins`, answered; never without a watcher.
async fn watcher_asked(origins: Option<&mut Watcher>) {
    match origins {
        Some(watcher) => watcher.asked().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests;
