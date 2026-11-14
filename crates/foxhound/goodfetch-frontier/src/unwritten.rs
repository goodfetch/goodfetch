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
//! commits are acknowledged the file holds it. A learned identity is then
//! forgotten at the next lock. A visited one stays, within
//! `run_state.memory.budget` (#2169): the file never takes a visit back,
//! so a duplicate check answers from memory and reads nothing, and past
//! the budget the visits the oldest batches wrote leave first and are read
//! from the file again. A chain's pending visit is held until its job
//! runs, or until its job is dropped unrun.
//!
//! As the crawl opens, memory reads the visits the file holds, within the
//! budget: once it read them all, and until one leaves it, memory holds
//! every visit, so a miss reads nothing either. Each lookup that reads the
//! file because memory does not hold every visit counts in
//! `goodfetch.run_state.spilled_lookups`, and what memory holds in
//! `goodfetch.run_state.memory`.
//!
//! The run-state writer tells these, as a [`WriteWatch`], as each job
//! begins and when one fails: its changes are then rolled back while its
//! batch goes on, so what it noted here is taken back too.
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

use goodfetch_engine::metrics::{Level, Metrics, Occurrences};
use goodfetch_engine::run_state::WriteWatch;
use goodfetch_protocol::ops::{RUN_STATE_MEMORY, RUN_STATE_SPILLED_LOOKUPS};
use rusqlite::Connection;

/// The visits the run file does not hold yet, the ones it holds that
/// memory keeps within its budget, and how many commits the writer
/// acknowledged, which says which of them it holds by now.
pub struct UnwrittenVisits {
    committed: Box<dyn Fn() -> u64 + Send + Sync>,
    held: Arc<Mutex<Held>>,
    /// Each lookup that read the file because memory does not hold every
    /// visit (`goodfetch.run_state.spilled_lookups`).
    spilled_lookups: Occurrences,
}

impl fmt::Debug for UnwrittenVisits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnwrittenVisits")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Held {
    /// Each identity visited, as the `visited` table keeps it.
    visited: HashMap<Arc<str>, Visit>,
    /// Each learned identity, with the rule that took its last parameter
    /// away, as the `learned_identities` table keeps it.
    learned: HashMap<Arc<str>, (i64, Visit)>,
    /// Each identity a writer job visited, in the order it did, with the
    /// commits acknowledged as it did: the order they reach the file.
    written: VecDeque<(u64, Space, Arc<str>)>,
    /// Each visited identity the file holds that memory keeps, in the
    /// order it reached the file, with its tag: the first leave first.
    resident: VecDeque<(u64, Arc<str>)>,
    /// What the writer job running now changed here, to take back if its
    /// work fails.
    journal: Journal,
    /// The bytes of what is held, as [`cost`] counts them.
    bytes: u64,
    /// The most bytes kept of what the file holds.
    budget: u64,
    /// The commits acknowledged as a seed last widened the scope.
    widened: Option<u64>,
    /// Whether memory read every visit the file held as the crawl opened.
    read_whole: bool,
    /// Whether a visit the file holds left memory past the budget.
    spilled: bool,
    /// The bytes held, as `goodfetch.run_state.memory` counts them.
    reported: Level,
}

/// Which of the visited set's tables an identity is held for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Visited,
    Learned,
}

#[derive(Debug)]
struct Visit {
    crawl_request: Box<str>,
    /// The commits acknowledged as a writer job visited it; none while
    /// its chain's visit job is queued.
    written: Option<u64>,
}

/// What one writer job changed, each identity with its tag before the
/// job: none when the job added it or held it as a chain's pending end.
#[derive(Debug, Default)]
struct Journal {
    /// The commits acknowledged as it noted its first change.
    tag: u64,
    changes: Vec<(Space, Arc<str>, Option<u64>)>,
}

/// The bytes an identity held in `space` for `crawl_request` costs: its
/// text, its map slot, its place in a queue and its shared header.
fn cost(space: Space, identity: &str, crawl_request: &str) -> u64 {
    let slot = match space {
        Space::Visited => size_of::<(Arc<str>, Visit)>(),
        Space::Learned => size_of::<(Arc<str>, (i64, Visit))>(),
    };
    let fixed = slot + size_of::<(u64, Space, Arc<str>)>() + 2 * size_of::<usize>();
    u64::try_from(identity.len() + crawl_request.len() + fixed).unwrap_or(u64::MAX)
}

impl Held {
    fn visit(&self, space: Space, identity: &str) -> Option<&Visit> {
        match space {
            Space::Visited => self.visited.get(identity),
            Space::Learned => self.learned.get(identity).map(|(_, visit)| visit),
        }
    }

    fn visit_mut(&mut self, space: Space, identity: &str) -> Option<&mut Visit> {
        match space {
            Space::Visited => self.visited.get_mut(identity),
            Space::Learned => self.learned.get_mut(identity).map(|(_, visit)| visit),
        }
    }

    /// Adds `identity` in `space`, for `crawl_request`, `written` under
    /// that tag or pending.
    fn insert(
        &mut self,
        space: Space,
        (identity, rule): (&Arc<str>, i64),
        crawl_request: &str,
        written: Option<u64>,
    ) {
        self.bytes = self
            .bytes
            .saturating_add(cost(space, identity, crawl_request));
        let visit = Visit {
            crawl_request: crawl_request.into(),
            written,
        };
        match space {
            Space::Visited => drop(self.visited.insert(Arc::clone(identity), visit)),
            Space::Learned => drop(self.learned.insert(Arc::clone(identity), (rule, visit))),
        }
    }

    /// Takes `identity` out of `space`.
    fn remove(&mut self, space: Space, identity: &str) {
        let removed = match space {
            Space::Visited => self.visited.remove(identity),
            Space::Learned => self.learned.remove(identity).map(|(_, visit)| visit),
        };
        if let Some(visit) = removed {
            self.bytes = self
                .bytes
                .saturating_sub(cost(space, identity, &visit.crawl_request));
        }
    }

    /// Notes that a writer job visited `identity` in `space` for
    /// `crawl_request` under `tag`, unless another request visited it
    /// first.
    fn write(
        &mut self,
        space: Space,
        (identity, rule): (&str, i64),
        crawl_request: &str,
        tag: u64,
    ) {
        if self.journal.tag != tag {
            // A job of an earlier batch noted these, and it ended.
            self.journal = Journal {
                tag,
                changes: Vec::new(),
            };
        }
        let identity: Arc<str> = match self.visit_mut(space, identity) {
            Some(visit) if *visit.crawl_request != *crawl_request => return,
            Some(visit) => {
                let before = visit.written.replace(tag);
                let identity = match space {
                    Space::Visited => self.visited.get_key_value(identity).map(|(key, _)| key),
                    Space::Learned => self.learned.get_key_value(identity).map(|(key, _)| key),
                }
                .map(Arc::clone)
                .expect("invariant: the identity just found is held");
                self.journal
                    .changes
                    .push((space, Arc::clone(&identity), before));
                identity
            }
            None => {
                let identity: Arc<str> = identity.into();
                self.insert(space, (&identity, rule), crawl_request, Some(tag));
                self.journal
                    .changes
                    .push((space, Arc::clone(&identity), None));
                identity
            }
        };
        self.written.push_back((tag, space, identity));
    }

    /// Takes back what the writer job running now changed: its work
    /// failed, so the file holds none of it.
    fn undo(&mut self) {
        let changes = std::mem::take(&mut self.journal.changes);
        for (space, identity, before) in changes.into_iter().rev() {
            match before {
                Some(tag) => {
                    if let Some(visit) = self.visit_mut(space, &identity) {
                        visit.written = Some(tag);
                    }
                }
                // Added by the job, or a chain's end its job visited: the
                // file holds neither, and the chain's job is over.
                None => self.remove(space, &identity),
            }
        }
    }

    /// Moves each identity a writer job visited in a batch the file holds
    /// once `committed` commits are acknowledged out of what the file
    /// does not hold: a learned one is forgotten, a visited one kept
    /// while what is held is within the budget, the oldest leaving first.
    fn prune(&mut self, committed: u64) {
        while let Some((tag, _, _)) = self.written.front()
            && *tag < committed
        {
            let Some((tag, space, identity)) = self.written.pop_front() else {
                break;
            };
            // A later visit of it holds a later tag, or none.
            if self
                .visit(space, &identity)
                .is_some_and(|visit| visit.written == Some(tag))
            {
                match space {
                    Space::Visited => self.resident.push_back((tag, identity)),
                    Space::Learned => self.remove(space, &identity),
                }
            }
        }
        while self.bytes > self.budget
            && let Some((tag, identity)) = self.resident.pop_front()
        {
            if self
                .visit(Space::Visited, &identity)
                .is_some_and(|visit| visit.written == Some(tag))
            {
                self.remove(Space::Visited, &identity);
                self.spilled = true;
            }
        }
    }

    /// Reads the visits `connection` holds, the file's, as the oldest
    /// memory keeps, until the next would take memory past the budget.
    fn read_visits(&mut self, connection: &Connection) -> rusqlite::Result<()> {
        let mut read = Vec::new();
        let mut whole = true;
        let mut statement = connection.prepare(VISITS)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let identity = row.get_ref(0)?.as_str()?;
            let crawl_request = row.get_ref(1)?.as_str()?;
            if let Some(visit) = self.visited.get_mut(identity) {
                // Held as a chain's pending end, which the file holds: it
                // stays once its job is dropped.
                if visit.written.is_none() {
                    visit.written = Some(0);
                    read.push(Arc::clone(
                        self.visited
                            .get_key_value(identity)
                            .map(|(key, _)| key)
                            .expect("invariant: the identity just found is held"),
                    ));
                }
                continue;
            }
            if self
                .bytes
                .saturating_add(cost(Space::Visited, identity, crawl_request))
                > self.budget
            {
                whole = false;
                break;
            }
            let identity: Arc<str> = identity.into();
            self.insert(Space::Visited, (&identity, 0), crawl_request, Some(0));
            read.push(identity);
        }
        // What the file held as the crawl opened reached it before any
        // batch memory noted: it leaves first.
        for identity in read.into_iter().rev() {
            self.resident.push_front((0, identity));
        }
        self.read_whole = whole;
        Ok(())
    }

    /// Whether memory holds every visit the file holds.
    const fn holds_every_visit(&self) -> bool {
        self.read_whole && !self.spilled
    }

    /// Counts the bytes held in `goodfetch.run_state.memory`.
    fn report(&mut self) {
        self.reported
            .set(usize::try_from(self.bytes).unwrap_or(usize::MAX));
    }
}

/// The statement [`Held::read_visits`] reads the file's visits with.
const VISITS: &str = "SELECT identity, crawl_request FROM visited";

/// The visits the run file does not hold yet, and those memory keeps,
/// locked for one duplicate check and the visit it decides.
pub(crate) struct Holding<'a> {
    visits: &'a UnwrittenVisits,
    held: MutexGuard<'a, Held>,
    committed: u64,
}

impl Drop for Holding<'_> {
    fn drop(&mut self) {
        self.held.report();
    }
}

impl Holding<'_> {
    /// The crawl request `identity` was visited for: memory's answer when
    /// it keeps the visit, or holds every visit the file holds; otherwise
    /// `read`'s from the file, counted as a spilled lookup
    /// (`goodfetch.run_state.spilled_lookups`).
    ///
    /// # Errors
    ///
    /// When `read` fails.
    pub(crate) fn visitor_or(
        &self,
        identity: &str,
        read: impl FnOnce() -> rusqlite::Result<Option<String>>,
    ) -> rusqlite::Result<Option<String>> {
        if let Some(visit) = self.held.visited.get(identity) {
            return Ok(Some(String::from(&*visit.crawl_request)));
        }
        if self.held.holds_every_visit() {
            return Ok(None);
        }
        self.visits.spilled_lookups.add(1);
        read()
    }

    /// The rule and crawl request of the learned identity `identity`,
    /// while the file may not hold it.
    pub(crate) fn learned(&self, identity: &str) -> Option<(i64, &str)> {
        self.held
            .learned
            .get(identity)
            .map(|(rule, visit)| (*rule, &*visit.crawl_request))
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
        self.held
            .write(Space::Visited, (plain, 0), crawl_request, tag);
        if let Some(learned) = learned {
            self.held.write(Space::Learned, learned, crawl_request, tag);
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
            let plain: Arc<str> = plain.into();
            held.insert(Space::Visited, (&plain, 0), crawl_request, None);
            inserted.push((Space::Visited, plain));
        }
        if let Some((learned, rule)) = learned
            && !held.learned.contains_key(learned)
        {
            let learned: Arc<str> = learned.into();
            held.insert(Space::Learned, (&learned, rule), crawl_request, None);
            inserted.push((Space::Learned, learned));
        }
        Pending {
            held: Arc::clone(&self.visits.held),
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
    inserted: Vec<(Space, Arc<str>)>,
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
            if held
                .visit(space, &identity)
                .is_some_and(|visit| visit.written.is_none())
            {
                held.remove(space, &identity);
            }
        }
        held.report();
    }
}

impl UnwrittenVisits {
    /// None held yet, keeping at most `budget` bytes of what the file
    /// holds, against a writer that has acknowledged `committed` commits
    /// as it is called, measured on `metrics`. Until it reads the file's
    /// visits ([`Self::read_visits`]), a miss reads the file.
    pub(crate) fn new(
        budget: u64,
        committed: impl Fn() -> u64 + Send + Sync + 'static,
        metrics: &Metrics,
    ) -> Self {
        Self {
            committed: Box::new(committed),
            held: Arc::new(Mutex::new(Held {
                visited: HashMap::new(),
                learned: HashMap::new(),
                written: VecDeque::new(),
                resident: VecDeque::new(),
                journal: Journal::default(),
                bytes: 0,
                budget,
                widened: None,
                read_whole: false,
                spilled: false,
                reported: metrics.level(&RUN_STATE_MEMORY),
            })),
            spilled_lookups: metrics.occurrences(&RUN_STATE_SPILLED_LOOKUPS),
        }
    }

    /// Reads the visits the file holds on `connection`, a reader's, as the
    /// crawl opens, within the budget: once memory read them all, and
    /// until one leaves it, a miss reads nothing. The file never takes a
    /// visit back, and each one written after these noted is held here
    /// until its batch commits, so none is missed.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn read_visits(&self, connection: &Connection) -> rusqlite::Result<()> {
        let mut holding = self.lock();
        holding.held.read_visits(connection)?;
        let committed = holding.committed;
        holding.held.prune(committed);
        Ok(())
    }

    /// The visits held, the ones the file holds by now forgotten or kept
    /// within the budget. Taken before a reader's first read, so its
    /// snapshot holds each one forgotten.
    pub(crate) fn lock(&self) -> Holding<'_> {
        let committed = (self.committed)();
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.prune(committed);
        Holding {
            visits: self,
            held,
            committed,
        }
    }

    /// The bytes held, as of the commits acknowledged now.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.lock().held.bytes
    }

    /// The watch the run-state writer tells as each of its jobs begins
    /// and when one fails ([`WriteWatch`]).
    #[must_use]
    pub fn watch(&self) -> Arc<dyn WriteWatch> {
        Arc::new(Watch(Arc::clone(&self.held)))
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

impl WriteWatch for UnwrittenVisits {
    fn began(&self) {
        Watch::began_on(&self.held);
    }

    fn failed(&self) {
        Watch::failed_on(&self.held);
    }
}

/// What the run-state writer tells of its jobs, shared with the visits
/// they note.
struct Watch(Arc<Mutex<Held>>);

impl fmt::Debug for Watch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Watch").finish_non_exhaustive()
    }
}

impl Watch {
    fn began_on(held: &Mutex<Held>) {
        held.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .journal
            .changes
            .clear();
    }

    fn failed_on(held: &Mutex<Held>) {
        let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
        held.undo();
        held.report();
    }
}

impl WriteWatch for Watch {
    fn began(&self) {
        Self::began_on(&self.0);
    }

    fn failed(&self) {
        Self::failed_on(&self.0);
    }
}

#[cfg(test)]
mod tests;
