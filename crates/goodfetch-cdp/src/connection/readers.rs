//! The readers of the connection's events: the [`Subscription`] its holder
//! reads, the list a subscription joins without the driver, the driver's
//! table of readers by session, and a reader's handout of the events it
//! reads to the subscriptions of the parts it reads for ([`Handout`],
//! [`Handing`]), as a Page's one reader hands each event of its sessions on
//! (#2265).
//!
//! Each event the driver takes in has its number, counted under the joining
//! list's lock: a subscription takes the events numbered after the count it
//! read as it started, so one that started before the driver took an event
//! in gets it, through a handout too, and one that started after does not,
//! though the reader handing it on had not reached it yet.
#![expect(
    clippy::disallowed_types,
    reason = "a joining list is shared by every handle and its driver or handing reader, each lock held for one list operation, never across an `.await`, and never inside another lock, so there is no lock order to test under loom; a subscriber's count of the bytes it holds is one counter, which orders no other memory (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_counted_work::{CountedWork, Row};
use tokio::sync::{mpsc, oneshot};

use super::{Event, SessionId};

/// How many events wait for a subscriber before the driver ends its
/// subscription: one that falls this far behind is no longer read, and the
/// driver never waits for it.
pub(super) const EVENTS: usize = 4_096;

/// The bytes of the events a subscriber may leave untaken, its quota: the
/// event that would take it past its quota ends its subscription, as one
/// [`EVENTS`] behind does. An event larger than the quota alone still
/// reaches a subscriber that holds none, so one that keeps up loses no event
/// and holds at most one message the framing reads
/// ([`MAX_FRAME`](crate::MAX_FRAME)).
pub(super) const EVENT_BYTES: usize = 64 * 1024 * 1024;

/// The bytes of the events one subscriber holds, which the driver adds to
/// and its holder takes from.
type Held = Arc<AtomicUsize>;

/// The events of one session, from when it was subscribed, or of each
/// session its reader was listed under since ([`MoreSessions`]).
#[derive(Debug)]
pub struct Subscription {
    events: mpsc::Receiver<Arc<Event>>,
    /// The bytes of the events it holds, which it takes from as its holder
    /// takes each.
    held: Held,
    /// Why the driver ended it, which it says before it drops its sender.
    why: oneshot::Receiver<SubscriptionEnd>,
    /// Why it ended, once known.
    ended: Option<SubscriptionEnd>,
    /// What lists it under more sessions.
    more: MoreSessions,
}

/// Why a subscription ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionEnd {
    /// The browser detached its session's target: the target sends nothing
    /// more.
    Detached,
    /// Its holder fell [`EVENTS`] events behind, or the events it left
    /// untaken came to more than its quota of bytes ([`EVENT_BYTES`]), so
    /// the events after it stopped are lost while its target may still send
    /// more.
    FellBehind,
    /// The connection ended.
    Disconnected,
}

impl Subscription {
    /// The next event; `None` once it ended: the connection ended, its
    /// session's target detached, or its holder fell behind, by
    /// [`EVENTS`] or by its quota of bytes, which [`ended`](Self::ended)
    /// tells apart.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub async fn next(&mut self) -> Option<Arc<Event>> {
        let event = self.events.recv().await;
        if let Some(event) = &event {
            self.held.fetch_sub(event.bytes(), Ordering::Relaxed);
        }
        event
    }

    /// Why it ended, once [`next`](Self::next) answered `None`; `None`
    /// while it goes on.
    pub fn ended(&mut self) -> Option<SubscriptionEnd> {
        if self.ended.is_none() && self.events.is_closed() && self.events.is_empty() {
            // The driver says why before it drops the events' sender, so a
            // sender dropped without a word is the connection's end.
            self.ended = Some(match self.why.try_recv() {
                Ok(why) => why,
                Err(_) => SubscriptionEnd::Disconnected,
            });
        }
        self.ended
    }

    /// The next event it holds now, without waiting: the fuzz target's.
    pub(super) fn taken(&mut self) -> Result<Arc<Event>, mpsc::error::TryRecvError> {
        let event = self.events.try_recv();
        if let Ok(event) = &event {
            self.held.fetch_sub(event.bytes(), Ordering::Relaxed);
        }
        event
    }

    /// What lists it under more sessions, beside the one it was subscribed
    /// to.
    pub fn more_sessions(&self) -> MoreSessions {
        self.more.clone()
    }
}

/// What lists a [`Subscription`] under more sessions than its first, as a
/// Page's one reader takes the events of its out-of-process frames and its
/// workers (spec #2256, "One reader per Page"): under each it shares the
/// subscription's one bounded buffer and one order. It never keeps the
/// subscription open: once its holder dropped it or it ended, it lists
/// nothing.
#[derive(Clone, Debug)]
pub struct MoreSessions {
    events: mpsc::WeakSender<Arc<Event>>,
    held: Held,
    /// The list it joins.
    joining: Shared,
    /// The list that numbers the events: the connection's.
    numbered: Shared,
}

impl MoreSessions {
    /// Lists the subscription under `session` (the browser's own when
    /// `None`) too, without waiting for the driver: it takes that session's
    /// events from now on. The detach of `session` takes this listing out
    /// alone; the subscription ends with its first session's detach, by
    /// falling behind, or with the connection. Its caller lists a session
    /// once.
    pub fn add(&self, session: Option<&SessionId>) {
        let after = lock(&self.numbered).read;
        let mut joining = lock(&self.joining);
        if joining.ended.is_some() {
            return;
        }
        // The driver ends a reader with all its listings under this lock, so
        // a reader that ended has no sender left to upgrade.
        let Some(events) = self.events.upgrade() else {
            return;
        };
        joining.list.push(Subscriber {
            session: session.cloned(),
            events,
            held: Arc::clone(&self.held),
            why: None,
            after,
        });
    }
}

/// One listing of a subscription, as the driver or a handing reader holds
/// it.
#[derive(Debug)]
struct Subscriber {
    /// The session whose events it takes.
    session: Option<SessionId>,
    events: mpsc::Sender<Arc<Event>>,
    /// The bytes of the events its holder has not taken, which the driver
    /// adds to as it hands each: one count for all of a subscription's
    /// listings.
    held: Held,
    /// Where the driver says why it ended the subscription: its first
    /// listing's alone.
    why: Option<oneshot::Sender<SubscriptionEnd>>,
    /// The events counted as it joined: it takes those numbered after.
    after: u64,
}

impl Subscriber {
    /// Ends it, saying `why` when it is its subscription's first listing.
    fn end(self, why: SubscriptionEnd) {
        if let Some(said) = self.why {
            let _ = said.send(why);
        }
    }

    /// Whether it is a listing of the subscription holding `held`.
    fn of(&self, held: &Held) -> bool {
        Arc::ptr_eq(&self.held, held)
    }
}

/// The subscriptions that joined since the driver, or a handing reader, last
/// took them in, which every handle adds to without it.
#[derive(Debug, Default)]
pub(super) struct Joining {
    /// Each subscription's session and the sender of its events.
    list: Vec<Subscriber>,
    /// Why every subscription ended, once they all did: the connection
    /// ended, or the reader handing them on did, so a new one ends at once.
    pub(super) ended: Option<SubscriptionEnd>,
    /// The events taken in so far, which number each.
    pub(super) read: u64,
}

/// A joining list, shared by every handle and the driver or the handing
/// reader.
pub(super) type Shared = Arc<Mutex<Joining>>;

/// The joining subscriptions, a panic in another holder notwithstanding:
/// each change is one list operation, so none leaves the list half changed.
pub(super) fn lock(joining: &Shared) -> MutexGuard<'_, Joining> {
    joining.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A subscription that joins `joining` for the events of `session`, those
/// `numbered` numbers after its count now; one that ends at once, saying
/// why, when every subscription of `joining` ended.
pub(super) fn join(
    joining: &Shared,
    numbered: &Shared,
    session: Option<&SessionId>,
) -> Subscription {
    let (events, receiver) = mpsc::channel(EVENTS);
    let (why, said) = oneshot::channel();
    let held = Held::default();
    let more = MoreSessions {
        events: events.downgrade(),
        held: Arc::clone(&held),
        joining: Arc::clone(joining),
        numbered: Arc::clone(numbered),
    };
    let after = lock(numbered).read;
    let mut list = lock(joining);
    match list.ended {
        // Said before its sender drops, as the driver does.
        Some(ended) => drop(why.send(ended)),
        None => list.list.push(Subscriber {
            session: session.cloned(),
            events,
            held: Arc::clone(&held),
            why: Some(why),
            after,
        }),
    }
    Subscription {
        events: receiver,
        held,
        why: said,
        ended: None,
        more,
    }
}

/// The readers of a stream of events by the session whose events each
/// takes: the driver's own table, or a handing reader's, which only its
/// owner reads and changes.
#[derive(Debug)]
pub(super) struct Readers {
    by_session: BTreeMap<Option<SessionId>, Vec<Subscriber>>,
    /// The bytes each may leave untaken: [`EVENT_BYTES`].
    quota: usize,
}

impl Readers {
    pub(super) fn new(quota: usize) -> Self {
        Self {
            by_session: BTreeMap::new(),
            quota,
        }
    }

    /// Takes in the subscriptions that joined `joining` since the last
    /// event, each under its session, whose readers their holders dropped
    /// leave first, so a session's list stays as long as its live readers;
    /// answers the number of the event taken in now.
    pub(super) fn take_in(&mut self, joining: &Shared) -> u64 {
        let (joined, number) = {
            let mut joining = lock(joining);
            joining.read += 1;
            (std::mem::take(&mut joining.list), joining.read)
        };
        for subscriber in joined {
            let readers = self
                .by_session
                .entry(subscriber.session.clone())
                .or_default();
            readers.retain(|reader| !reader.events.is_closed());
            readers.push(subscriber);
        }
        number
    }

    /// Hands `event` to the readers of its session alone that joined before
    /// it was numbered, each visit counted on `work` when given, never
    /// waiting for one: a reader its holder dropped leaves, and one that
    /// fell behind ends, [`EVENTS`] behind or by the event that would take
    /// it past its quota while it holds others, with all its listings.
    pub(super) fn hand(
        &mut self,
        event: &Arc<Event>,
        work: Option<&CountedWork>,
        joining: &Shared,
    ) {
        let std::collections::btree_map::Entry::Occupied(mut entry) =
            self.by_session.entry(event.session().cloned())
        else {
            return;
        };
        let bytes = event.bytes();
        let mut fell_behind = Vec::new();
        for reader in std::mem::take(entry.get_mut()) {
            if event.number() <= reader.after {
                entry.get_mut().push(reader);
                continue;
            }
            if let Some(work) = work {
                work.count(Row::CdpEventVisits);
            }
            // Only its holder takes from it meanwhile, so the bytes it holds
            // are at most those read here.
            let held = reader.held.load(Ordering::Relaxed);
            if held > 0 && held.saturating_add(bytes) > self.quota {
                tracing::warn!(
                    method = event.method,
                    "a subscription of the browser's connection left more bytes untaken than its quota and ends"
                );
                fell_behind.push(reader);
                continue;
            }
            reader.held.fetch_add(bytes, Ordering::Relaxed);
            match reader.events.try_send(Arc::clone(event)) {
                Ok(()) => entry.get_mut().push(reader),
                Err(mpsc::error::TrySendError::Closed(_)) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    reader.held.fetch_sub(bytes, Ordering::Relaxed);
                    tracing::warn!(
                        method = event.method,
                        "a subscription of the browser's connection fell too far behind and ends"
                    );
                    fell_behind.push(reader);
                }
            }
        }
        if entry.get().is_empty() {
            entry.remove();
        }
        for reader in fell_behind {
            self.end_reader(reader, SubscriptionEnd::FellBehind, joining);
        }
    }

    /// Ends the readers of the session `gone` the browser detached, saying
    /// so, and forgets the session: a subscription first listed under it
    /// ends with all its listings, and one listed under it among more
    /// sessions loses that listing alone.
    pub(super) fn detached(&mut self, gone: &SessionId, joining: &Shared) {
        for reader in self
            .by_session
            .remove(&Some(gone.clone()))
            .into_iter()
            .flatten()
        {
            if reader.why.is_some() {
                self.end_reader(reader, SubscriptionEnd::Detached, joining);
            }
        }
    }

    /// Ends the subscription `reader` lists, saying `why`, with each of its
    /// listings, in this table and joining `joining`: under the joining
    /// list's lock, so none joins after.
    fn end_reader(&mut self, reader: Subscriber, why: SubscriptionEnd, joining: &Shared) {
        let mut joining = lock(joining);
        let held = Arc::clone(&reader.held);
        let mut listings = vec![reader];
        listings.extend(joining.list.extract_if(.., |joined| joined.of(&held)));
        self.by_session.retain(|_, readers| {
            listings.extend(readers.extract_if(.., |listed| listed.of(&held)));
            !readers.is_empty()
        });
        for listing in listings {
            listing.end(why);
        }
    }

    /// Ends every reader, saying `why`, and the joining list `joining`, so a
    /// subscription that joins after ends at once.
    pub(super) fn end(self, why: SubscriptionEnd, joining: &Shared) {
        let joined = {
            let mut joining = lock(joining);
            joining.ended = Some(why);
            std::mem::take(&mut joining.list)
        };
        for reader in joined
            .into_iter()
            .chain(self.by_session.into_values().flatten())
        {
            reader.end(why);
        }
    }
}

/// Where the parts a reader reads for subscribe to the events it hands on,
/// as they would to the connection's: a part subscribes to a session the
/// reader takes, and gets its events the connection took in after it
/// subscribed. Any task may clone it.
#[derive(Clone, Debug)]
pub struct Handout {
    joining: Shared,
    /// The connection's joining list, which numbers its events.
    numbered: Shared,
}

impl Handout {
    /// The events of `session` (the browser's own when `None`) its reader
    /// hands on from now on, without waiting for it; it ends when its
    /// session's target detaches, when its holder falls [`EVENTS`] events
    /// behind, or when its reader's handing ends, saying why.
    pub fn subscribe(&self, session: Option<&SessionId>) -> Subscription {
        join(&self.joining, &self.numbered, session)
    }
}

/// What a reader hands the events it reads on with, to the subscriptions
/// its [`Handout`] gave, by their session as the driver hands its own; its
/// visits are its own work, not the connection's (`C1.event_visits`).
#[derive(Debug)]
pub struct Handing {
    readers: Readers,
    joining: Shared,
}

impl Handing {
    /// The handing of the events the connection whose joining list is
    /// `numbered` numbers, and its handout.
    pub(super) fn new(numbered: &Shared) -> (Handout, Self) {
        let joining = Shared::default();
        (
            Handout {
                joining: Arc::clone(&joining),
                numbered: Arc::clone(numbered),
            },
            Self {
                readers: Readers::new(EVENT_BYTES),
                joining,
            },
        )
    }

    /// Hands `event` to the subscriptions of its session that started
    /// before the connection took it in, never waiting for one.
    pub fn hand(&mut self, event: &Arc<Event>) {
        self.readers.take_in(&self.joining);
        self.readers.hand(event, None, &self.joining);
    }

    /// Ends the subscriptions of the session `gone` the browser detached,
    /// saying so.
    pub fn detached(&mut self, gone: &SessionId) {
        self.readers.take_in(&self.joining);
        self.readers.detached(gone, &self.joining);
    }

    /// Ends every subscription it hands to, saying `why`, as the reader
    /// ended; one that subscribes after ends at once, saying the same.
    pub fn end(self, why: SubscriptionEnd) {
        self.readers.end(why, &self.joining);
    }
}
