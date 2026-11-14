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
//! The identities themselves, and the checks that read them, are
//! `identity`'s: this module only holds what they decided.

#![expect(
    clippy::disallowed_types,
    reason = "the visits a batching writer has not committed, guarded for a duplicate check and its visit, inside one writer job or one reader transaction, never held with another lock or across an await, so there is no lock order to test under loom"
)]

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
    shared: &'a Arc<Mutex<Held>>,
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

    /// Notes that a writer job visited `plain` and, when the learned
    /// rules changed it, `learned` with the rule that took its last
    /// parameter away, for `crawl_request` in the batch it runs in, which
    /// the file holds once one more commit is acknowledged.
    pub(crate) fn written(
        &mut self,
        plain: &str,
        learned: Option<(&str, i64)>,
        crawl_request: &str,
    ) {
        let tag = self.committed;
        let held = &mut *self.held;
        let visit = held
            .visited
            .entry(plain.to_owned())
            .or_insert_with(|| Visit::pending(crawl_request.to_owned()));
        if visit.crawl_request == crawl_request {
            visit.written = Some(tag);
            held.written
                .push_back((tag, Space::Visited, plain.to_owned()));
        }
        if let Some((learned, rule)) = learned {
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

    /// Holds `plain` and, when the learned rules changed it, `learned`
    /// with its rule, as visited for `crawl_request` by a chain's end
    /// whose visit job is queued, until the [`Pending`] it answers is
    /// done or dropped.
    pub(crate) fn hold(
        &mut self,
        plain: &str,
        learned: Option<(&str, i64)>,
        crawl_request: &str,
    ) -> Pending {
        let mut inserted = Vec::new();
        let held = &mut *self.held;
        if !held.visited.contains_key(plain) {
            held.visited
                .insert(plain.to_owned(), Visit::pending(crawl_request.to_owned()));
            inserted.push((Space::Visited, plain.to_owned()));
        }
        if let Some((learned, rule)) = learned
            && !held.learned.contains_key(learned)
        {
            held.learned.insert(
                learned.to_owned(),
                (rule, Visit::pending(crawl_request.to_owned())),
            );
            inserted.push((Space::Learned, learned.to_owned()));
        }
        Pending {
            held: Arc::clone(self.shared),
            inserted,
            done: false,
        }
    }

    /// Notes that a writer job widened the scope in the batch it runs in.
    pub(crate) fn widened(&mut self) {
        self.held.widened = Some(self.committed);
    }
}

/// A chain's end held as visited until its visit job runs, which marks it
/// done, or is dropped unrun, which takes what it added away.
pub(crate) struct Pending {
    held: Arc<Mutex<Held>>,
    /// What this chain's end added.
    inserted: Vec<(Space, String)>,
    done: bool,
}

impl Pending {
    /// Its visit job ran: what it held stays, as a writer job's visit.
    pub(crate) fn done(mut self) {
        self.done = true;
    }
}

impl Drop for Pending {
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
        Holding {
            shared: &self.held,
            held,
            committed,
        }
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
}

#[cfg(test)]
mod tests;
