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
//! commits are acknowledged the file holds it. It then stays, within
//! `run_state.memory.budget` (#2169): the file never takes a visit back,
//! so a duplicate check answers from memory and reads nothing, and past
//! the budget the visits the oldest batches wrote leave first and are read
//! from the file again. A learned identity stays by the same rule, and is
//! read again as a rule that merges changes (`learned`). A chain's
//! pending visit is held until its job runs, or until its job is dropped
//! unrun.
//!
//! As the crawl opens, memory reads the seeds, then the visits, the file
//! holds, within the budget: once it read them all, and until one leaves it, memory holds
//! every visit, so a miss reads nothing either. Each lookup that reads the
//! file because memory does not hold every visit counts in
//! `goodfetch.run_state.spilled_lookups`, and what memory holds in
//! `goodfetch.run_state.memory`.
//!
//! The Bounds' counts, the requests admitted against `crawl.max_requests`
//! and each path budget's, are held here too (#2169): read from the file
//! once, on the writer, by the first admission that writes, then counted
//! here as each admission writes its own to the file, so no later one
//! reads them. A staged admission, rolled back, counts beside them and
//! never in them.
//!
//! The Scope's seeds are held here as the visited identities are (#2169),
//! by the measure the Run's strategy compares: each seed a writer job
//! widens the scope with, and those the file holds, read as the crawl
//! opens, within the same budget, leaving memory by the same rule. A
//! link's scope check then reads the file only for a seed that left it,
//! and counts that lookup in `goodfetch.run_state.spilled_lookups` too.
//!
//! The learned rules are held here too (#2169), each as the file holds
//! it, read once by the first admission that writes and again, from the
//! lowest rule the writer changed, by the next one after a change; they
//! count in the budget and never leave memory (`rules`).
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
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_engine::metrics::{Level, Metrics, Occurrences};
use goodfetch_engine::run_state::WriteWatch;
use goodfetch_protocol::ops::{RUN_STATE_MEMORY, RUN_STATE_SPILLED_LOOKUPS};
use rusqlite::Connection;

pub(crate) use self::learned::{EVERY_LEARNED, LEARNED_WITHIN};
pub(crate) use self::rules::{HeldRule, LearnedRules};

mod learned;
mod rules;

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
    /// Each seed's measure, as the `scope_seeds` table keeps it, with no
    /// crawl request.
    seeds: HashMap<Arc<str>, Visit>,
    /// Each identity or seed a writer job wrote, in the order it did,
    /// with the commits acknowledged as it did: the order they reach the
    /// file.
    written: VecDeque<(u64, Space, Arc<str>)>,
    /// Each visited identity and seed the file holds that memory keeps,
    /// in the order it reached the file, with its tag: the first leave
    /// first.
    resident: VecDeque<(u64, Space, Arc<str>)>,
    /// The Bounds' counts, as the file holds them with the writes not
    /// committed yet, once an admission that writes read them.
    counts: Option<Counts>,
    /// The learned rules, once an admission that writes read them.
    rules: Option<LearnedRules>,
    /// The lowest ID of a rule the writer changed since memory read them,
    /// or [`i64::MAX`]: told by the writer's connection as it changes one.
    rules_changed: Arc<AtomicI64>,
    /// What the writer job running now changed here, to take back if its
    /// work fails.
    journal: Journal,
    /// The bytes the entries held allocate, as [`cost`] counts them; the
    /// tables and queues holding them add theirs ([`Held::bytes`]).
    entries: u64,
    /// The most bytes kept of what the file holds.
    budget: u64,
    /// The commits acknowledged as a seed last widened the scope.
    widened: Option<u64>,
    /// Whether memory holds every visit the file holds.
    every_visit: Whole,
    /// Whether memory holds every seed the file holds.
    every_seed: Whole,
    /// Whether memory holds every learned identity the file holds.
    every_learned: Whole,
    /// Whether memory read the learned identities the file holds.
    learned_read: bool,
    /// The bytes held, as `goodfetch.run_state.memory` counts them.
    reported: Level,
}

/// Which of the run file's tables an identity, or a seed's measure, is
/// held for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Visited,
    Learned,
    Seed,
}

/// Whether memory holds every entry of one table the file holds: it read
/// them all as the crawl opened, and none left it since.
#[derive(Clone, Copy, Debug, Default)]
struct Whole {
    read: bool,
    spilled: bool,
}

impl Whole {
    const fn holds(self) -> bool {
        self.read && !self.spilled
    }
}

#[derive(Debug)]
struct Visit {
    /// The crawl request it was visited for; empty for a seed's measure.
    crawl_request: Box<str>,
    /// The commits acknowledged as a writer job visited it; none while
    /// its chain's visit job is queued.
    written: Option<u64>,
}

/// The Bounds' counts the Frontier keeps in memory (#2169): the requests
/// admitted, and each path budget's, by its pattern, as the run file holds
/// them with the writes not committed yet. Written with the batch and
/// never removed; the refusals' counts are only written.
#[derive(Clone, Debug, Default)]
pub(crate) struct Counts {
    admitted: u64,
    paths: HashMap<Box<str>, u64>,
}

impl Counts {
    /// The counts of `admitted` requests, and of each path budget's, by
    /// its pattern.
    pub(crate) const fn new(admitted: u64, paths: HashMap<Box<str>, u64>) -> Self {
        Self { admitted, paths }
    }

    /// The requests admitted.
    pub(crate) const fn admitted(&self) -> u64 {
        self.admitted
    }

    /// The requests the path budget of `pattern` admitted.
    pub(crate) fn path_admitted(&self, pattern: &str) -> u64 {
        self.paths.get(pattern).copied().unwrap_or_default()
    }

    /// Counts one request admitted, against the path budget of `pattern`
    /// when one counts it.
    fn count(&mut self, pattern: Option<&str>) {
        self.admitted = self.admitted.saturating_add(1);
        if let Some(pattern) = pattern {
            let path = self.paths.entry(pattern.into()).or_default();
            *path = path.saturating_add(1);
        }
    }
}

/// What one writer job changed, each identity with its tag before the
/// job: none when the job added it or held it as a chain's pending end;
/// and the Bounds' counts before its first count, when it counted.
#[derive(Debug, Default)]
struct Journal {
    /// The commits acknowledged as it noted its first change.
    tag: u64,
    changes: Vec<(Space, Arc<str>, Option<u64>)>,
    counts: CountsBefore,
    /// The lowest ID of the learned rules it read, which its failure has
    /// memory read again.
    rules_from: Option<i64>,
    /// Whether it read every learned identity, which its failure has
    /// memory read again.
    learned_read: bool,
}

/// The Bounds' counts memory held before a writer job first changed them.
#[derive(Debug, Default)]
enum CountsBefore {
    /// The job changed none.
    #[default]
    Unchanged,
    /// Memory held none: the job read them from the file.
    Unread,
    /// Memory held these.
    Held(Counts),
}

impl Journal {
    /// Forgets what a job of an earlier batch noted, which ended, once a
    /// job notes a change under `tag`.
    fn under(&mut self, tag: u64) -> &mut Self {
        if self.tag != tag {
            *self = Self {
                tag,
                ..Self::default()
            };
        }
        self
    }

    /// Forgets what the job that ran before noted.
    fn clear(&mut self) {
        self.changes.clear();
        self.counts = CountsBefore::Unchanged;
        self.rules_from = None;
        self.learned_read = false;
    }
}

/// The bytes an identity held for `crawl_request` allocates of its own:
/// its text behind its shared header, and the request's text. Its slot in
/// a table and its places in the queues are the containers' bytes
/// ([`Held::containers`]).
fn cost(identity: &str, crawl_request: &str) -> u64 {
    u64::try_from(2 * size_of::<usize>() + identity.len() + crawl_request.len()).unwrap_or(u64::MAX)
}

/// The bytes a hash table with room for `capacity` entries of `T` takes,
/// as std's lays it out: for each bucket a slot and a control byte, one
/// bucket in eight kept empty past seven, and a group of control bytes
/// more. A table never gives back the room it grew to, and grows past
/// what it holds as entries leave and others come, so its room, not its
/// entries, is what it costs.
fn table<T>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = if capacity < 8 {
        capacity + 1
    } else {
        capacity / 7 * 8
    };
    buckets * (size_of::<T>() + 1) + CONTROL_GROUP
}

/// The control bytes a hash table holds past its buckets.
const CONTROL_GROUP: usize = 16;

/// The room a container holding `len` entries keeps once it gives back
/// what it does not need: an eighth more, and one, so the next entries
/// fill it before it grows. Without it a full memory would give back its
/// room at one lock and grow again at the next admission, rebuilding a
/// table each time; with it, at most once in an eighth of what it holds.
fn ample(len: usize) -> usize {
    if len == 0 { 0 } else { len + len / 8 + 1 }
}

/// The room a hash table asked for room for `len` entries keeps, as std's
/// sizes it: a power of two of buckets,
/// one in eight kept empty past eight.
fn room(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let buckets = match len {
        1..4 => 4,
        4..8 => 8,
        _ => (len.saturating_mul(8) / 7).next_power_of_two(),
    };
    if buckets <= 8 {
        buckets - 1
    } else {
        buckets / 8 * 7
    }
}

impl Held {
    fn visit(&self, space: Space, identity: &str) -> Option<&Visit> {
        match space {
            Space::Visited => self.visited.get(identity),
            Space::Learned => self.learned.get(identity).map(|(_, visit)| visit),
            Space::Seed => self.seeds.get(identity),
        }
    }

    /// The key `identity` is held under in `space`.
    fn key(&self, space: Space, identity: &str) -> Option<&Arc<str>> {
        match space {
            Space::Visited => self.visited.get_key_value(identity),
            Space::Learned => self
                .learned
                .get_key_value(identity)
                .map(|(key, (_, visit))| (key, visit)),
            Space::Seed => self.seeds.get_key_value(identity),
        }
        .map(|(key, _)| key)
    }

    fn visit_mut(&mut self, space: Space, identity: &str) -> Option<&mut Visit> {
        match space {
            Space::Visited => self.visited.get_mut(identity),
            Space::Learned => self.learned.get_mut(identity).map(|(_, visit)| visit),
            Space::Seed => self.seeds.get_mut(identity),
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
        self.entries = self.entries.saturating_add(cost(identity, crawl_request));
        let visit = Visit {
            crawl_request: crawl_request.into(),
            written,
        };
        match space {
            Space::Visited => drop(self.visited.insert(Arc::clone(identity), visit)),
            Space::Learned => drop(self.learned.insert(Arc::clone(identity), (rule, visit))),
            Space::Seed => drop(self.seeds.insert(Arc::clone(identity), visit)),
        }
    }

    /// Takes `identity` out of `space`.
    fn remove(&mut self, space: Space, identity: &str) {
        let removed = match space {
            Space::Visited => self.visited.remove(identity),
            Space::Learned => self.learned.remove(identity).map(|(_, visit)| visit),
            Space::Seed => self.seeds.remove(identity),
        };
        if let Some(visit) = removed {
            self.entries = self
                .entries
                .saturating_sub(cost(identity, &visit.crawl_request));
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
        self.journal.under(tag);
        let identity: Arc<str> = match self.visit_mut(space, identity) {
            Some(visit) if *visit.crawl_request != *crawl_request => return,
            Some(visit) => {
                let before = visit.written.replace(tag);
                let identity = self
                    .key(space, identity)
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
        if let Some(from) = self.journal.rules_from.take() {
            self.rules_changed.fetch_min(from, Ordering::SeqCst);
        }
        if std::mem::take(&mut self.journal.learned_read) {
            self.learned_read = false;
            self.every_learned = Whole::default();
        }
        match std::mem::take(&mut self.journal.counts) {
            CountsBefore::Unchanged => {}
            CountsBefore::Unread => self.counts = None,
            CountsBefore::Held(counts) => self.counts = Some(counts),
        }
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

    /// Moves each identity a writer job visited, and each seed it wrote,
    /// in a batch the file holds once `committed` commits are acknowledged
    /// out of what the file does not hold: each is kept while what is held
    /// is within the budget, the oldest leaving first.
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
                self.resident.push_back((tag, space, identity));
            }
        }
        self.fit();
    }

    /// Brings what is held within the budget: takes what the file holds
    /// out of memory, the oldest first, until it fits, or would fit were
    /// the containers to keep only the room they need, then gives back the
    /// room they keep past that when it alone keeps them past the budget.
    /// Room grows by doubling, so a table can outgrow the budget while its
    /// entries fit.
    fn fit(&mut self) {
        if self.bytes() <= self.budget {
            return;
        }
        self.evict();
        if self.bytes() > self.budget {
            self.visited.shrink_to(ample(self.visited.len()));
            self.learned.shrink_to(ample(self.learned.len()));
            self.seeds.shrink_to(ample(self.seeds.len()));
            self.written.shrink_to(ample(self.written.len()));
            self.resident.shrink_to(ample(self.resident.len()));
        }
    }

    /// Takes what the file holds out of memory, the oldest first, until
    /// what is held would be within the budget in containers keeping only
    /// the room they need, or memory holds nothing the file holds.
    fn evict(&mut self) {
        while self
            .entries
            .saturating_add(self.kept())
            .saturating_add(self.containers().min(self.least(0)))
            > self.budget
            && let Some((tag, space, identity)) = self.resident.pop_front()
        {
            if self
                .visit(space, &identity)
                .is_some_and(|visit| visit.written == Some(tag))
            {
                self.remove(space, &identity);
                match space {
                    Space::Visited => self.every_visit.spilled = true,
                    Space::Learned => self.every_learned.spilled = true,
                    Space::Seed => self.every_seed.spilled = true,
                }
            }
        }
    }

    /// The bytes held: the entries' own ([`cost`]), the containers' and
    /// the learned rules'.
    fn bytes(&self) -> u64 {
        self.entries
            .saturating_add(self.containers())
            .saturating_add(self.kept())
    }

    /// The bytes the learned rules take, which never leave memory.
    fn kept(&self) -> u64 {
        self.rules.as_ref().map_or(0, LearnedRules::bytes)
    }

    /// The bytes the tables and queues holding the entries take, by the
    /// room each has ([`table`]).
    fn containers(&self) -> u64 {
        let queued = self.written.capacity() + self.resident.capacity();
        let bytes = table::<(Arc<str>, Visit)>(self.visited.capacity())
            + table::<(Arc<str>, (i64, Visit))>(self.learned.capacity())
            + table::<(Arc<str>, Visit)>(self.seeds.capacity())
            + queued * size_of::<(u64, Space, Arc<str>)>();
        u64::try_from(bytes).unwrap_or(u64::MAX)
    }

    /// The bytes the containers would take keeping only the room they
    /// need ([`ample`]), with `queued` more in the resident queue.
    fn least(&self, queued: usize) -> u64 {
        let queued = ample(self.written.len()) + ample(self.resident.len() + queued);
        let bytes = table::<(Arc<str>, Visit)>(room(ample(self.visited.len())))
            + table::<(Arc<str>, (i64, Visit))>(room(ample(self.learned.len())))
            + table::<(Arc<str>, Visit)>(room(ample(self.seeds.len())))
            + queued * size_of::<(u64, Space, Arc<str>)>();
        u64::try_from(bytes).unwrap_or(u64::MAX)
    }

    /// Reads the seeds `connection` holds, the file's, each as `seeds`
    /// reads its measure, then its visits, as the oldest memory keeps,
    /// until the next would take memory past the budget: the visits leave
    /// first, then the seeds, which every link's scope check asks. With no
    /// `seeds` to read, under `all`, memory holds every seed it needs.
    fn read(&mut self, connection: &Connection, seeds: Option<&str>) -> rusqlite::Result<()> {
        let (seeds, every_seed) = match seeds {
            Some(query) => self.read_space(connection, Space::Seed, query, 0)?,
            None => (Vec::new(), true),
        };
        let (visits, every_visit) =
            self.read_space(connection, Space::Visited, VISITS, seeds.len())?;
        // What the file held as the crawl opened reached it before any
        // batch memory noted: it leaves first, the visits before the
        // seeds.
        for (space, read) in [(Space::Seed, seeds), (Space::Visited, visits)] {
            for identity in read.into_iter().rev() {
                self.resident.push_front((0, space, identity));
            }
        }
        self.every_seed.read = every_seed;
        self.every_visit.read = every_visit;
        self.fit();
        Ok(())
    }

    /// Reads into `space` the rows `query` reads on `connection`, each a
    /// visited identity and its crawl request, or a seed's measure, until
    /// the next would take memory past the budget, `queued` read before
    /// it to be queued with them; answers those it read, and whether it
    /// read them all.
    fn read_space(
        &mut self,
        connection: &Connection,
        space: Space,
        query: &str,
        queued: usize,
    ) -> rusqlite::Result<(Vec<Arc<str>>, bool)> {
        let mut read = Vec::new();
        let mut statement = connection.prepare(query)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let identity = row.get_ref(0)?.as_str()?;
            let crawl_request = match space {
                Space::Visited => row.get_ref(1)?.as_str()?,
                Space::Learned | Space::Seed => "",
            };
            if let Some(visit) = self.visit_mut(space, identity) {
                // Held as a chain's pending end, which the file holds: it
                // stays once its job is dropped.
                if visit.written.is_none() {
                    visit.written = Some(0);
                    read.push(Arc::clone(
                        self.key(space, identity)
                            .expect("invariant: the identity just found is held"),
                    ));
                }
                continue;
            }
            let identity: Arc<str> = identity.into();
            self.insert(space, (&identity, 0), crawl_request, Some(0));
            if self
                .entries
                .saturating_add(self.kept())
                .saturating_add(self.least(queued + read.len() + 1))
                > self.budget
            {
                self.remove(space, &identity);
                return Ok((read, false));
            }
            read.push(identity);
        }
        Ok((read, true))
    }

    /// Counts the bytes held in `goodfetch.run_state.memory`.
    fn report(&mut self) {
        self.reported
            .set(usize::try_from(self.bytes()).unwrap_or(usize::MAX));
    }
}

/// The statement [`Held::read`] reads the file's visits with.
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
        if self.held.every_visit.holds() {
            return Ok(None);
        }
        self.visits.spilled_lookups.add(1);
        read()
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

    /// The Bounds' counts memory holds: as `read` reads them from the
    /// writer's connection, by the first call of an admission that
    /// `writes`; none before it, when the counts are the file's.
    ///
    /// # Errors
    ///
    /// When `read` cannot read the run file.
    pub(crate) fn counts(
        &mut self,
        writes: bool,
        read: impl FnOnce() -> rusqlite::Result<Counts>,
    ) -> rusqlite::Result<Option<&Counts>> {
        if self.held.counts.is_none() && writes {
            let counts = read()?;
            let held = &mut *self.held;
            let journal = held.journal.under(self.committed);
            if matches!(journal.counts, CountsBefore::Unchanged) {
                journal.counts = CountsBefore::Unread;
            }
            held.counts = Some(counts);
        }
        Ok(self.held.counts.as_ref())
    }

    /// Counts one request a writer job admitted, against the path budget
    /// of `pattern` when one counts it, as it writes the count to the
    /// file in the batch it runs in.
    pub(crate) fn counted(&mut self, pattern: Option<&str>) {
        let held = &mut *self.held;
        if let Some(counts) = &mut held.counts {
            let journal = held.journal.under(self.committed);
            if matches!(journal.counts, CountsBefore::Unchanged) {
                journal.counts = CountsBefore::Held(counts.clone());
            }
            counts.count(pattern);
        }
    }

    /// Notes that a writer job widened the scope in the batch it runs in,
    /// and holds `seed`, the measure of the seed it widened it with, when
    /// the job writes it to the file.
    pub(crate) fn widened(&mut self, seed: Option<&str>) {
        self.held.widened = Some(self.committed);
        if let Some(seed) = seed {
            self.held.write(Space::Seed, (seed, 0), "", self.committed);
        }
    }

    /// Whether a seed of one of `measures` reaches a link: memory's
    /// answer when it holds one, or holds every seed the file holds;
    /// otherwise `read`'s from the file, counted as a spilled lookup
    /// (`goodfetch.run_state.spilled_lookups`).
    ///
    /// # Errors
    ///
    /// When `read` fails.
    pub(crate) fn seed_or<'m>(
        &self,
        measures: impl IntoIterator<Item = &'m str>,
        read: impl FnOnce() -> rusqlite::Result<bool>,
    ) -> rusqlite::Result<bool> {
        if measures
            .into_iter()
            .any(|measure| self.held.seeds.contains_key(measure))
        {
            return Ok(true);
        }
        if self.held.every_seed.holds() {
            return Ok(false);
        }
        self.visits.spilled_lookups.add(1);
        read()
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
    /// seeds and visits ([`Self::read`]), a miss reads the file.
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
                seeds: HashMap::new(),
                written: VecDeque::new(),
                resident: VecDeque::new(),
                counts: None,
                rules: None,
                rules_changed: rules::unchanged(),
                journal: Journal::default(),
                entries: 0,
                budget,
                widened: None,
                every_visit: Whole::default(),
                every_seed: Whole::default(),
                every_learned: Whole::default(),
                learned_read: false,
                reported: metrics.level(&RUN_STATE_MEMORY),
            })),
            spilled_lookups: metrics.occurrences(&RUN_STATE_SPILLED_LOOKUPS),
        }
    }

    /// Reads the seeds the file holds on `connection`, a reader's, each
    /// as `seeds` reads its measure, then its visits, as the crawl opens,
    /// within the budget: once memory read them all, and until one leaves
    /// it, a miss reads nothing. The file never takes a visit or a seed
    /// back, and each one written after these noted is held here until
    /// its batch commits, so none is missed.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub(crate) fn read(
        &self,
        connection: &Connection,
        seeds: Option<&str>,
    ) -> rusqlite::Result<()> {
        let mut holding = self.lock();
        holding.held.read(connection, seeds)?;
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
        self.lock().held.bytes()
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
