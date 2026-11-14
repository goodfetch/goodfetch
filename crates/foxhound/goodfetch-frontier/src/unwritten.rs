//! The visits the run file does not hold yet, held in memory beside it
//! while the run-state writer batches (#2168): each identity a writer job
//! visited in a batch not committed yet, and each one a redirect chain
//! reached whose visit job is queued but has not run. A duplicate check
//! reads the run file and these under one lock, so a reader's snapshot,
//! which holds every batch committed before it began, and these together
//! are never behind the writer: a redirect hop is then judged on a reader,
//! and its page waits for no write.
//!
//! An identity the writer visited is tagged with the commits the writer
//! had acknowledged as its job ran: its batch is the next, so once more
//! commits are acknowledged the file holds it, and the next lock forgets
//! it. A chain's pending visit is held until its job runs, or until its
//! job is dropped unrun.
//!
//! Nothing here is held at `full`, `fast` or `periodic`, where each
//! attempt's writes are a batch of their own and a hop asks the writer.

#![expect(
    clippy::disallowed_types,
    reason = "the visits a batching writer has not committed, guarded for a duplicate check and its visit, inside one writer job or one reader transaction, never held with another lock or across an await, so there is no lock order to test under loom"
)]

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rusqlite::Connection;
use url::Url;

use crate::identity::{self, Declared, Hop, Identities, Key, Seen};
use crate::invariants::Invariants;

/// The visits the run file does not hold yet, and how many commits the
/// writer acknowledged, which says which of them it holds by now.
pub struct UnwrittenVisits {
    committed: Box<dyn Fn() -> u64 + Send + Sync>,
    held: Arc<Mutex<Held>>,
}

impl fmt::Debug for UnwrittenVisits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnwrittenVisits")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct Held {
    /// Each identity visited, as the `visited` table keeps it.
    visited: HashMap<String, Visit>,
    /// Each learned identity, with the rule that took its last parameter
    /// away, as the `learned_identities` table keeps it.
    learned: HashMap<String, (i64, Visit)>,
    /// Each identity a writer job visited, in the order it did, with the
    /// commits acknowledged as it did: the order they reach the file.
    written: VecDeque<(u64, Space, String)>,
    /// The commits acknowledged as a seed last widened the scope.
    widened: Option<u64>,
}

/// Which of the visited set's tables an identity is held for.
#[derive(Clone, Copy, Debug)]
enum Space {
    Visited,
    Learned,
}

#[derive(Debug)]
struct Visit {
    crawl_request: String,
    /// The commits acknowledged as a writer job visited it; none while
    /// its chain's visit job is queued.
    written: Option<u64>,
}

impl Visit {
    const fn pending(crawl_request: String) -> Self {
        Self {
            crawl_request,
            written: None,
        }
    }
}

impl Held {
    /// Forgets each identity a writer job visited in a batch the file
    /// holds once `committed` commits are acknowledged.
    fn prune(&mut self, committed: u64) {
        while let Some((tag, _, _)) = self.written.front()
            && *tag < committed
        {
            let Some((tag, space, identity)) = self.written.pop_front() else {
                break;
            };
            let visit = match space {
                Space::Visited => self.visited.get(&identity),
                Space::Learned => self.learned.get(&identity).map(|(_, visit)| visit),
            };
            // A later visit of it holds a later tag, or none.
            if visit.is_some_and(|visit| visit.written == Some(tag)) {
                match space {
                    Space::Visited => drop(self.visited.remove(&identity)),
                    Space::Learned => drop(self.learned.remove(&identity)),
                }
            }
        }
    }
}

/// The visits the run file does not hold yet, locked for one duplicate
/// check and the visit it decides.
pub(crate) struct Holding<'a> {
    held: MutexGuard<'a, Held>,
    committed: u64,
}

impl Holding<'_> {
    /// The crawl request `identity` was visited for, while the file may
    /// not hold it.
    pub(crate) fn visitor(&self, identity: &str) -> Option<&str> {
        self.held
            .visited
            .get(identity)
            .map(|visit| visit.crawl_request.as_str())
    }

    /// The rule and crawl request of the learned identity `identity`,
    /// while the file may not hold it.
    pub(crate) fn learned(&self, identity: &str) -> Option<(i64, &str)> {
        self.held
            .learned
            .get(identity)
            .map(|(rule, visit)| (*rule, visit.crawl_request.as_str()))
    }

    /// Notes that a writer job visited `identities` for `crawl_request`
    /// in the batch it runs in, which the file holds once one more commit
    /// is acknowledged.
    pub(crate) fn written(&mut self, identities: &Identities<'_>, crawl_request: &str) {
        let tag = self.committed;
        let held = &mut *self.held;
        let visit = held
            .visited
            .entry(identities.plain().to_owned())
            .or_insert_with(|| Visit::pending(crawl_request.to_owned()));
        if visit.crawl_request == crawl_request {
            visit.written = Some(tag);
            held.written
                .push_back((tag, Space::Visited, identities.plain().to_owned()));
        }
        if let Some((learned, rule)) = identities.learned() {
            let (_, visit) = held
                .learned
                .entry(learned.to_owned())
                .or_insert_with(|| (rule, Visit::pending(crawl_request.to_owned())));
            if visit.crawl_request == crawl_request {
                visit.written = Some(tag);
                held.written
                    .push_back((tag, Space::Learned, learned.to_owned()));
            }
        }
    }

    /// Notes that a writer job widened the scope in the batch it runs in.
    pub(crate) fn widened(&mut self) {
        self.held.widened = Some(self.committed);
    }
}

/// What a redirect chain's end found, judged on a reader.
#[derive(Debug)]
pub enum Reached {
    /// Another request was admitted for, or reached, its key: the request
    /// ends `skipped` with `duplicate`.
    Duplicate,
    /// The request goes on: its key is held as pending, until the visit
    /// job made of it ([`visit_pending`]) runs.
    Follow(PendingVisit),
}

/// A chain's end, held as visited until its visit job runs or is dropped
/// unrun.
pub struct PendingVisit {
    held: Arc<Mutex<Held>>,
    crawl_request: String,
    identities: Identities<'static>,
    /// What this chain's end added, which a drop unrun takes away.
    inserted: Vec<(Space, String)>,
    done: bool,
}

impl fmt::Debug for PendingVisit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingVisit")
            .field("crawl_request", &self.crawl_request)
            .finish_non_exhaustive()
    }
}

impl Drop for PendingVisit {
    fn drop(&mut self) {
        if self.done || self.inserted.is_empty() {
            return;
        }
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        for (space, identity) in self.inserted.drain(..) {
            let pending = match space {
                Space::Visited => held.visited.get(&identity),
                Space::Learned => held.learned.get(&identity).map(|(_, visit)| visit),
            }
            .is_some_and(|visit| visit.written.is_none());
            if pending {
                match space {
                    Space::Visited => drop(held.visited.remove(&identity)),
                    Space::Learned => drop(held.learned.remove(&identity)),
                }
            }
        }
    }
}

impl UnwrittenVisits {
    /// None held yet, against a writer that has acknowledged `committed`
    /// commits as it is called.
    pub(crate) fn new(committed: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            committed: Box::new(committed),
            held: Arc::default(),
        }
    }

    /// The visits held, the ones the file holds by now forgotten. Taken
    /// before a reader's first read, so its snapshot holds each one
    /// forgotten.
    pub(crate) fn lock(&self) -> Holding<'_> {
        let committed = (self.committed)();
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.prune(committed);
        Holding { held, committed }
    }

    /// Whether a seed widened the scope in a batch not committed before
    /// `committed` commits were acknowledged: a reader that began after
    /// them may not have seen it.
    #[must_use]
    pub fn widened_since(&self, committed: u64) -> bool {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .widened
            .is_some_and(|widened| widened >= committed)
    }

    /// Where the redirect chain of the crawl request `crawl_request` ends,
    /// at `url`, sent with `method` and `body`, judged on the reader
    /// `connection` as [`identity::redirect_reached`] judges it: on
    /// `Follow`, its identities are held as pending, so no admission or
    /// other chain takes them before its visit job runs.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn hold_reached(
        &self,
        connection: &Connection,
        crawl_request: &str,
        (url, method, body): (&Url, &HttpMethod, Option<&[u8]>),
    ) -> rusqlite::Result<Reached> {
        let mut holding = self.lock();
        let reached = reached_identities(connection, url, method, body)?;
        if duplicate(
            &identity::seen(connection, &reached, None, Some(&holding))?,
            crawl_request,
        ) {
            return Ok(Reached::Duplicate);
        }
        let mut inserted = Vec::new();
        let held = &mut *holding.held;
        if !held.visited.contains_key(reached.plain()) {
            held.visited.insert(
                reached.plain().to_owned(),
                Visit::pending(crawl_request.to_owned()),
            );
            inserted.push((Space::Visited, reached.plain().to_owned()));
        }
        if let Some((learned, rule)) = reached.learned()
            && !held.learned.contains_key(learned)
        {
            held.learned.insert(
                learned.to_owned(),
                (rule, Visit::pending(crawl_request.to_owned())),
            );
            inserted.push((Space::Learned, learned.to_owned()));
        }
        Ok(Reached::Follow(PendingVisit {
            held: Arc::clone(&self.held),
            crawl_request: crawl_request.to_owned(),
            identities: reached,
            inserted,
            done: false,
        }))
    }
}

/// Visits `pending`, a chain's end held as pending, on the writer's
/// `connection`, as [`identity::redirect_reached`] visits it.
///
/// # Errors
///
/// When the run file cannot be written.
pub fn visit_pending(
    connection: &Connection,
    invariants: &Invariants,
    mut pending: PendingVisit,
) -> rusqlite::Result<()> {
    if let Some(unwritten) = invariants.unwritten() {
        let mut holding = unwritten.lock();
        identity::visit_all(connection, &pending.identities, &pending.crawl_request)?;
        holding.written(&pending.identities, &pending.crawl_request);
        pending.done = true;
        drop(holding);
    } else {
        identity::visit_all(connection, &pending.identities, &pending.crawl_request)?;
        pending.done = true;
    }
    Ok(())
}

/// The identities of a redirect chain's end at `url`, sent with `method`
/// and `body`.
pub(crate) fn reached_identities(
    connection: &Connection,
    url: &Url,
    method: &HttpMethod,
    body: Option<&[u8]>,
) -> rusqlite::Result<Identities<'static>> {
    identity::identities(
        connection,
        &Key::of(url, false),
        Declared {
            method,
            body,
            key: None,
            keeps_fragment: false,
        },
    )
}

/// Whether `seen` makes a hop of `crawl_request` a `duplicate`: another
/// request was admitted for, or reached, its key.
pub(crate) fn duplicate(seen: &Seen, crawl_request: &str) -> bool {
    match seen {
        Seen::Visited(visitor)
        | Seen::Merged {
            crawl_request: visitor,
            ..
        } => visitor != crawl_request,
        Seen::New => false,
    }
}

/// The hop answer of a chain's end that `duplicate` says.
pub(crate) const fn reached_hop(duplicate: bool) -> Hop {
    if duplicate {
        Hop::Duplicate
    } else {
        Hop::Follow
    }
}

#[cfg(test)]
mod tests;
