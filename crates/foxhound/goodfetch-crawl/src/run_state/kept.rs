//! What the crawl keeps of its pending work items (#2169): each change to
//! one that the run file's readers may not see yet ([`Known`]), and, at a
//! level that batches attempts, while they fit within the Run's memory
//! budget (`run_state.memory.budget`) beside the Frontier's visits, every
//! pending work item whole ([`Whole`]), so a lease reads nothing from the
//! run file. Once they no longer fit, they spill for good: the crawl reads
//! the pending work items from the run file again, beside the changes it
//! keeps, which it went on keeping all along.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};

use goodfetch_engine::admission::Waiting;
use goodfetch_frontier::{SharedBudget, hash_table_bytes};
use rusqlite::Connection;
use serde_json::Value;

use crate::work_items::{self, Again, Leased};

/// A work item, as its lease reads what the crawl kept of it.
pub(super) enum Leasing {
    /// Known pending: its lease takes this row.
    Kept(Box<Leased>),
    /// Leased, ended or merged: no lease takes it.
    Unleasable,
    /// Nothing kept: a read finds it as it is.
    Unknown,
}

/// What the crawl knows of a work item that the run file's readers may not
/// see yet: at a level that batches attempts, a write is answered once it
/// is in the open batch (ADR 0142), and a claim is queued and not waited
/// for at all (#2168).
pub(super) enum Known {
    /// Its claim is queued and its attempt not settled: it is leased.
    InFlight,
    /// It ended, or a rule merged it, once `.0` commits were acknowledged.
    Gone(u64),
    /// It waits as `.0` says its next lease takes it, which a write
    /// answered once `.1` commits were acknowledged: a retry, or a work
    /// item an attempt added.
    Pending(Box<Leased>, u64),
}

/// The work items whose latest change the run file's readers may not see
/// yet, each as [`Known`] says, and every pending one whole while they fit
/// within the Run's memory budget. A change answered once `committed`
/// commits were acknowledged is in a batch that commits after them, so a
/// read that starts once more have been acknowledged sees it.
pub(super) struct Kept {
    known: HashMap<String, Known>,
    whole: Option<Whole>,
}

impl Kept {
    /// Keeps every pending work item of `whole`, when the crawl opened
    /// with them all in memory.
    pub(super) fn new(whole: Option<Whole>) -> Self {
        Self {
            known: HashMap::new(),
            whole,
        }
    }

    /// Forgets each change a read that starts once `committed` commits
    /// were acknowledged sees: a claim stays until its attempt settles.
    pub(super) fn seen(&mut self, committed: u64) {
        self.known.retain(|_, known| match known {
            Known::InFlight => true,
            Known::Gone(answered) | Known::Pending(_, answered) => *answered >= committed,
        });
    }

    /// Forgets every change but the claims: the open batch is in the file.
    pub(super) fn sealed(&mut self) {
        self.known
            .retain(|_, known| matches!(known, Known::InFlight));
    }

    /// Whether a read may still find `id` pending though it is not.
    pub(super) fn holds(&self, id: &str) -> bool {
        matches!(self.known.get(id), Some(Known::InFlight | Known::Gone(_)))
    }

    /// How many work items a read may still find pending though they are
    /// not.
    pub(super) fn held(&self) -> usize {
        self.known
            .values()
            .filter(|known| matches!(known, Known::InFlight | Known::Gone(_)))
            .count()
    }

    /// Each work item pending of `key`, or of every key when `None`, that
    /// a read may not find so.
    pub(super) fn pending<'a>(&'a self, key: Option<&'a str>) -> impl Iterator<Item = &'a Waiting> {
        self.known.values().filter_map(move |known| match known {
            Known::Pending(leased, _) if key.is_none_or(|key| leased.waiting.key == key) => {
                Some(&leased.waiting)
            }
            _ => None,
        })
    }

    /// Takes the change kept of `id`, as its lease reads it.
    pub(super) fn leasing(&mut self, id: &str) -> Leasing {
        match self.known.remove(id) {
            None => Leasing::Unknown,
            Some(Known::Pending(leased, _)) => Leasing::Kept(leased),
            Some(known) => {
                self.known.insert(id.to_owned(), known);
                Leasing::Unleasable
            }
        }
    }

    /// Keeps `id` gone once `committed` commits were acknowledged, unless
    /// its attempt is in flight, whose settling says how it ended.
    pub(super) fn gone(&mut self, id: String, committed: u64) {
        if let Some(whole) = &mut self.whole {
            whole.remove(&id);
        }
        if !matches!(self.known.get(&id), Some(Known::InFlight)) {
            self.known.insert(id, Known::Gone(committed));
        }
    }

    /// Keeps the attempt of `leased` settled once `committed` commits were
    /// acknowledged: its work item pending again as its next lease takes
    /// it, retried or released as `again` says, else ended. Pending again,
    /// it spills every pending work item once they no longer fit within
    /// the budget beside the Frontier's visits, which hold `beside` bytes.
    pub(super) fn settled(
        &mut self,
        leased: Leased,
        again: Again,
        committed: u64,
        beside: impl FnOnce() -> u64,
    ) {
        let id = leased.crawl_request.id.clone();
        let known = match leased.after(again) {
            Some(next) => {
                self.keep_whole(|whole| whole.insert(&next, beside()));
                Known::Pending(Box::new(next), committed)
            }
            None => Known::Gone(committed),
        };
        self.known.insert(id, known);
    }

    /// Keeps `leased` pending, as its next lease takes it, in a write
    /// answered once `committed` commits were acknowledged; answers how it
    /// waits for its domain's queue. Spills every pending work item once
    /// they no longer fit within the budget beside the Frontier's visits,
    /// which hold `beside` bytes.
    pub(super) fn added(
        &mut self,
        leased: Leased,
        committed: u64,
        beside: impl FnOnce() -> u64,
    ) -> Waiting {
        self.keep_whole(|whole| whole.insert(&leased, beside()));
        let waiting = leased.waiting.clone();
        self.known.insert(
            waiting.id.clone(),
            Known::Pending(Box::new(leased), committed),
        );
        waiting
    }

    /// Keeps `id` leased until its attempt settles.
    pub(super) fn claimed(&mut self, id: String) {
        if let Some(whole) = &mut self.whole {
            whole.remove(&id);
        }
        self.known.insert(id, Known::InFlight);
    }

    /// Every pending work item, while they fit within the Run's memory
    /// budget beside the Frontier's visits, which hold `beside` bytes:
    /// once they do not, they spill for good.
    pub(super) fn whole(&mut self, beside: impl FnOnce() -> u64) -> Option<&Whole> {
        if self
            .whole
            .as_ref()
            .is_some_and(|whole| !whole.within(beside()))
        {
            self.spill();
        }
        self.whole.as_ref()
    }

    /// Lets every pending work item go from memory for good: a pending one
    /// the crawl did not hear of, or more than fit within the Run's memory
    /// budget. The run file's reads find them, beside the changes kept.
    pub(super) fn spill(&mut self) {
        if self.whole.take().is_some() {
            tracing::debug!("the crawl's pending work items spilled from memory to the run file");
        }
    }

    /// Changes the whole pending work items with `change`, which answers
    /// whether they still fit; spills them when they do not.
    fn keep_whole(&mut self, change: impl FnOnce(&mut Whole) -> bool) {
        if self.whole.as_mut().is_some_and(|whole| !change(whole)) {
            self.spill();
        }
    }
}

/// Where a work item waits in its domain's queue: `forefront` ones latest
/// first, then highest priority first, then in order, as
/// [`in_queue_order`](super::in_queue_order) sorts them.
type Place = (Reverse<Option<i64>>, Reverse<i64>, i64, String);

/// The place of `waiting` in its domain's queue.
fn place(waiting: &Waiting) -> Place {
    (
        Reverse(waiting.forefront.then_some(waiting.order)),
        Reverse(waiting.priority),
        waiting.order,
        waiting.id.clone(),
    )
}

/// The least memory holds of a pending work item: its row's slot and its
/// place's in its queue.
const ROW_BYTES: usize = size_of::<(String, Leased)>() + size_of::<Place>();

/// The text memory holds of `leased`: its ID's four copies, in its row's
/// key, its request, how it waits and its place, and the rest of its
/// row's, its Payload's included.
fn text(leased: &Leased) -> usize {
    let request = &leased.crawl_request;
    4 * request.id.len()
        + request.url.len()
        + request.label.len()
        + request.method.as_str().len()
        + leased.waiting.key.len()
        + request.body.as_ref().map_or(0, |body| body.bytes().len())
        + request.payload.as_ref().map_or(0, value_bytes)
}

/// The bytes a leaf of a queue's tree takes, as std's `BTreeSet` lays it
/// out: its parent, its index in it and its length, and room for eleven
/// places.
const LEAF_BYTES: usize = (size_of::<usize>() + 2 * size_of::<u16>() + 11 * size_of::<Place>())
    .next_multiple_of(align_of::<Place>());

/// The bytes an inner node of a queue's tree takes: a leaf's, and room
/// for twelve edges.
const INNER_BYTES: usize = LEAF_BYTES + 12 * size_of::<usize>();

/// The bytes the tree of a queue of `len` work items takes, at most. One
/// leaf holds up to eleven places; past them, a node holds at least five,
/// each beside the place its parent keeps between it and the next, as an
/// insert splits it or a removal merges it: a leaf for each six places,
/// and an inner node for each five leaves, the nodes above them counted.
const fn tree(len: usize) -> usize {
    match len {
        0 => 0,
        1..=11 => LEAF_BYTES,
        _ => {
            let leaves = len.div_ceil(6);
            leaves * LEAF_BYTES + leaves.div_ceil(5) * INNER_BYTES
        }
    }
}

/// The bytes memory holds of `value`, roughly.
fn value_bytes(value: &Value) -> usize {
    size_of::<Value>()
        + match value {
            Value::String(text) => text.len(),
            Value::Array(items) => items.iter().map(value_bytes).sum(),
            Value::Object(fields) => fields
                .iter()
                .map(|(name, value)| size_of::<String>() + name.len() + value_bytes(value))
                .sum(),
            Value::Null | Value::Bool(_) | Value::Number(_) => 0,
        }
}

/// Every pending work item that is not leased, as its next lease takes it,
/// each in its domain's queue, within the Run's memory budget, which it
/// shares with the Frontier's visits: it tells the budget the bytes it
/// holds as they change, and none once it is dropped. Like the Frontier's
/// ([`hash_table_bytes`]), it counts its maps by their room, not by their
/// entries.
pub(crate) struct Whole {
    rows: HashMap<String, Leased>,
    queues: HashMap<String, BTreeSet<Place>>,
    /// The bytes held past the maps' tables: the rows' text, and each
    /// queue's key and tree.
    held: usize,
    budget: SharedBudget,
}

impl Drop for Whole {
    fn drop(&mut self) {
        self.budget.pending_hold(0);
    }
}

impl Whole {
    /// Every pending work item the run file holds as the crawl opens, when
    /// they fit within `budget` beside the Frontier's visits, which hold
    /// `beside` bytes; `None` when they do not, or when one has no domain
    /// key yet.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) fn read(
        connection: &Connection,
        budget: SharedBudget,
        beside: u64,
    ) -> rusqlite::Result<Option<Self>> {
        let room =
            budget.bytes().saturating_sub(beside) / u64::try_from(ROW_BYTES).unwrap_or(u64::MAX);
        // One more than fit tells that they do not.
        let most = usize::try_from(room)
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        let ids = work_items::pending(connection, most)?;
        if ids.len() == most {
            return Ok(None);
        }
        let mut whole = Self {
            rows: HashMap::with_capacity(ids.len()),
            queues: HashMap::new(),
            held: 0,
            budget,
        };
        for leased in work_items::leasable(connection, &ids)? {
            if !whole.insert(&leased, beside) {
                return Ok(None);
            }
        }
        Ok(whole.within(beside).then_some(whole))
    }

    /// Keeps `leased` pending in its domain's queue, in place of any row
    /// kept of it; answers whether they all still fit within the budget
    /// beside the Frontier's visits, which hold `beside` bytes, and
    /// whether it has a domain key.
    fn insert(&mut self, leased: &Leased, beside: u64) -> bool {
        let key = &leased.waiting.key;
        if key.is_empty() {
            return false;
        }
        self.remove(&leased.crawl_request.id);
        let queue = self.queues.entry(key.clone()).or_default();
        let len = queue.len();
        queue.insert(place(&leased.waiting));
        self.held += text(leased) + tree(len + 1) - tree(len);
        if len == 0 {
            self.held += key.len();
        }
        self.rows
            .insert(leased.crawl_request.id.clone(), leased.clone());
        self.budget.pending_hold(self.bytes());
        self.within(beside)
    }

    /// Lets `id` go: it is no longer pending.
    fn remove(&mut self, id: &str) {
        let Some(leased) = self.rows.remove(id) else {
            return;
        };
        self.held -= text(&leased);
        let key = &leased.waiting.key;
        if let Some(queue) = self.queues.get_mut(key) {
            let len = queue.len();
            queue.remove(&place(&leased.waiting));
            self.held -= tree(len) - tree(len - 1);
            if queue.is_empty() {
                self.held -= key.len();
                self.queues.remove(key);
            }
        }
        self.budget.pending_hold(self.bytes());
    }

    /// The bytes they hold: past their text and trees, the tables of the
    /// two maps, by their room.
    fn bytes(&self) -> u64 {
        let tables = hash_table_bytes::<(String, Leased)>(self.rows.capacity())
            + hash_table_bytes::<(String, BTreeSet<Place>)>(self.queues.capacity());
        u64::try_from(self.held + tables).unwrap_or(u64::MAX)
    }

    /// Whether they all fit within the budget beside `beside` bytes.
    fn within(&self, beside: u64) -> bool {
        self.bytes().saturating_add(beside) <= self.budget.bytes()
    }

    /// The pending work items of `key`, or of every key when `None`, at
    /// most `bound` of each key, in the order its queue admits them.
    pub(super) fn waiting(&self, key: Option<&str>, bound: usize) -> Vec<Waiting> {
        let queue = |queue: &BTreeSet<Place>| {
            queue
                .iter()
                .take(bound)
                .filter_map(|place| self.rows.get(&place.3))
                .map(|leased| leased.waiting.clone())
                .collect::<Vec<_>>()
        };
        match key {
            Some(key) => self.queues.get(key).map(queue).unwrap_or_default(),
            None => self.queues.values().flat_map(queue).collect(),
        }
    }

    /// The IDs of at most `limit` pending work items.
    pub(super) fn pending(&self, limit: usize) -> Vec<String> {
        self.rows.keys().take(limit).cloned().collect()
    }

    /// Each pending work item of `ids`, by ID, as its next lease takes it.
    pub(super) fn leasable(&self, ids: &[String]) -> HashMap<String, Leased> {
        ids.iter()
            .filter_map(|id| Some((id.clone(), self.rows.get(id)?.clone())))
            .collect()
    }
}

#[cfg(test)]
mod tests;
