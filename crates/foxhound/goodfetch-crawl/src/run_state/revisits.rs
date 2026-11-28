//! In-memory and spilled revisit schedule owned by the crawl's Run state
//! (ADR 0139, ADR 0142).
//!
//! On a batch crawl (`continuous == false`), this tracker is [`Disabled`] and
//! performs zero allocations and zero queries.
//!
//! On a Continuous crawl (`continuous == true`), it tracks every visited URL's
//! [`RevisitRow`] and due order in memory while within `run_state.memory.budget`,
//! and spills to the run file's `revisits` table (with an unwritten-batch
//! overlay) when the budget is 0 or exceeded — reading bounded windows through
//! the covering index and counting each reader fill on
//! `F3.revisits_due_window_reads`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use goodfetch_engine::admission::Waiting;
use goodfetch_frontier::SharedBudget;
use goodfetch_frontier::revisit::{self, DueEntry, PreviousState, RevisitRow};
use goodfetch_protocol::foxhound::outcomes::OutcomeKind;
use rusqlite::Connection;

use crate::work_items::{CountedAs, Leased};

/// Estimated bytes held in memory for one cached revisit entry.
const ENTRY_BYTES: u64 = 256;

/// What the Run state holds of its Continuous crawl revisits.
pub(super) struct RevisitTracker {
    /// Whether `crawl.continuous` is enabled for this Run.
    enabled: bool,
    /// Full in-memory schedule while within `run_state.memory.budget`; `None`
    /// once spilled (or when `budget == 0`), in which case due windows are
    /// read from the run file through a reader.
    whole: Option<WholeRevisits>,
    /// Overlay of revisit rows written in open Run-state batches not yet
    /// acknowledged by readers, keyed by `work_item` ID with the commit count
    /// at which the write ran.
    unwritten: BTreeMap<String, (RevisitRow, u64)>,
    /// Revisits currently enqueued in `admission` or leased in flight: maps
    /// `work_item` ID to `(optional cached Leased, previous CountedAs)`.
    active: HashMap<String, ActiveRevisit>,
    /// Cached `Leased` templates for work items that finished, kept only while
    /// `whole` is `Some` so in-memory revisits need 0 SQL reads to lease.
    templates: HashMap<String, Leased>,
}

/// A revisit currently waiting in its domain queue or leased in flight.
pub(super) struct ActiveRevisit {
    /// Cached `Leased` row prepared for the revisit attempt, when held in memory.
    pub(super) leased: Option<Leased>,
    /// Previous outcome counted in `Tally` before this revisit was claimed.
    pub(super) previous_outcome: CountedAs,
    /// Whether this revisit has already been claimed into flight (not just
    /// waiting in `admission`'s domain queue).
    pub(super) in_flight: bool,
}

/// Full in-memory revisit schedule within the Run's memory budget.
struct WholeRevisits {
    budget: SharedBudget,
    bytes: u64,
    rows: HashMap<String, RevisitRow>,
    /// Global due index: `(next_visit_at_ms, domain_key, work_item_id)`.
    due_global: BTreeSet<(u64, String, String)>,
    /// Per-domain due index: `domain_key -> {(next_visit_at_ms, work_item_id)}`.
    due_by_domain: BTreeMap<String, BTreeSet<(u64, String)>>,
}

impl WholeRevisits {
    fn new(budget: SharedBudget) -> Self {
        Self {
            budget,
            bytes: 0,
            rows: HashMap::new(),
            due_global: BTreeSet::new(),
            due_by_domain: BTreeMap::new(),
        }
    }

    fn within(&self, beside: u64) -> bool {
        let cap = self.budget.bytes();
        cap > 0 && self.bytes.saturating_add(beside) <= cap
    }

    fn remove_due(&mut self, row: &RevisitRow) {
        if let Some(due) = row.next_visit_at {
            self.due_global
                .remove(&(due, row.domain_key.clone(), row.work_item.clone()));
            if let Some(domain_set) = self.due_by_domain.get_mut(&row.domain_key) {
                domain_set.remove(&(due, row.work_item.clone()));
                if domain_set.is_empty() {
                    self.due_by_domain.remove(&row.domain_key);
                }
            }
        }
    }

    fn insert(&mut self, row: RevisitRow, beside: u64) -> bool {
        let is_new = !self.rows.contains_key(&row.work_item);
        if let Some(old) = self.rows.remove(&row.work_item) {
            self.remove_due(&old);
        }
        if let Some(due) = row.next_visit_at {
            self.due_global
                .insert((due, row.domain_key.clone(), row.work_item.clone()));
            self.due_by_domain
                .entry(row.domain_key.clone())
                .or_default()
                .insert((due, row.work_item.clone()));
        }
        self.rows.insert(row.work_item.clone(), row);
        if is_new {
            self.bytes = self.bytes.saturating_add(ENTRY_BYTES);
        }
        self.within(beside)
    }

    fn earliest_due(&self, active: &HashMap<String, ActiveRevisit>) -> Option<u64> {
        self.due_global
            .iter()
            .find(|(_, _, id)| !active.contains_key(id))
            .map(|(due, _, _)| *due)
    }

    fn due_window(
        &self,
        domain_key: Option<&str>,
        now_ms: u64,
        limit: usize,
        active: &HashMap<String, ActiveRevisit>,
    ) -> Vec<DueEntry> {
        match domain_key {
            Some(domain) => self
                .due_by_domain
                .get(domain)
                .into_iter()
                .flat_map(|set| set.iter())
                .take_while(|(due, _)| *due <= now_ms)
                .filter(|(_, id)| !active.contains_key(id))
                .take(limit)
                .map(|(due, id)| DueEntry {
                    work_item: id.clone(),
                    domain_key: domain.to_owned(),
                    next_visit_at: *due,
                })
                .collect(),
            None => self
                .due_global
                .iter()
                .take_while(|(due, _, _)| *due <= now_ms)
                .filter(|(_, _, id)| !active.contains_key(id))
                .take(limit)
                .map(|(due, domain, id)| DueEntry {
                    work_item: id.clone(),
                    domain_key: domain.clone(),
                    next_visit_at: *due,
                })
                .collect(),
        }
    }
}

/// Opening state read once when a crawl opens/resumes.
pub(crate) struct OpenedRevisits {
    enabled: bool,
    whole: Option<WholeRevisits>,
}

impl OpenedRevisits {
    /// Disabled tracker for batch crawls (zero queries, zero allocations).
    pub(super) fn disabled() -> Self {
        Self {
            enabled: false,
            whole: None,
        }
    }

    /// Reads persisted `revisits` rows on Continuous crawl open/resume,
    /// keeping them in memory when within `budget` beside `beside` bytes.
    ///
    /// # Errors
    ///
    /// Returns SQLite errors from reading `revisits`.
    pub(super) fn read(
        connection: &Connection,
        budget: Option<SharedBudget>,
        beside: u64,
    ) -> rusqlite::Result<Self> {
        let Some(budget) = budget else {
            return Ok(Self {
                enabled: true,
                whole: None,
            });
        };
        let cap = budget.bytes();
        if cap == 0 || beside > cap {
            return Ok(Self {
                enabled: true,
                whole: None,
            });
        }
        let rows = revisit::read_all(connection)?;
        let mut whole = WholeRevisits::new(budget);
        for row in rows {
            if !whole.insert(row, beside) {
                return Ok(Self {
                    enabled: true,
                    whole: None,
                });
            }
        }
        Ok(Self {
            enabled: true,
            whole: Some(whole),
        })
    }
}

impl RevisitTracker {
    /// Builds the tracker from [`OpenedRevisits`].
    pub(super) fn from_opened(opened: OpenedRevisits) -> Self {
        Self {
            enabled: opened.enabled,
            whole: opened.whole,
            unwritten: BTreeMap::new(),
            active: HashMap::new(),
            templates: HashMap::new(),
        }
    }

    /// Whether Continuous crawl revisit tracking is active.
    pub(super) const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Drops unwritten-batch overlay entries that readers can now see.
    pub(super) fn seen(&mut self, committed: u64) {
        self.unwritten.retain(|_, (_, wrote)| *wrote >= committed);
    }

    /// Drops all unwritten-batch overlay entries after `seal()`.
    pub(super) fn sealed(&mut self) {
        self.unwritten.clear();
    }

    /// Returns previous schedule state for `work_item` from memory if known.
    pub(super) fn previous_state(&self, work_item: &str) -> Option<PreviousState> {
        self.unwritten
            .get(work_item)
            .map(|(row, _)| row)
            .or_else(|| self.whole.as_ref().and_then(|w| w.rows.get(work_item)))
            .map(|row| PreviousState {
                interval_ms: row.interval_ms,
                visits: row.visits,
                changes: row.changes,
            })
    }

    /// Records a committed attempt's updated [`RevisitRow`] and caches its
    /// [`Leased`] template while in memory.
    pub(super) fn record_committed(
        &mut self,
        leased: Leased,
        row: RevisitRow,
        committed: u64,
        beside: impl FnOnce() -> u64,
    ) {
        if !self.enabled {
            return;
        }
        let id = row.work_item.clone();
        self.active.remove(&id);
        self.unwritten.insert(id.clone(), (row.clone(), committed));
        if let Some(whole) = &mut self.whole {
            if whole.insert(row, beside()) {
                self.templates.insert(id, leased);
            } else {
                self.whole = None;
                self.templates.clear();
            }
        }
    }

    /// Marks an unsuccessful revisit attempt as no longer active in flight.
    pub(super) fn settled_unsuccessful(&mut self, work_item: &str) {
        self.active.remove(work_item);
    }

    /// Whether the tracker holds its full schedule in memory (0 SQL reads).
    pub(super) fn holds_whole(&mut self, beside: impl FnOnce() -> u64) -> bool {
        if self
            .whole
            .as_ref()
            .is_some_and(|whole| !whole.within(beside()))
        {
            self.whole = None;
            self.templates.clear();
        }
        self.whole.is_some()
    }

    /// Earliest due timestamp in memory (when `whole` is `Some`), or overlay
    /// minimum merged with a reader's `from_disk` when spilled.
    pub(super) fn earliest_due_in_memory(&self) -> Option<u64> {
        self.whole.as_ref()?.earliest_due(&self.active)
    }

    /// Merges a reader's earliest due timestamp with the unwritten overlay.
    pub(super) fn merge_earliest_due(&self, from_disk: Option<u64>) -> Option<u64> {
        let overlay = self
            .unwritten
            .iter()
            .filter(|(id, _)| !self.active.contains_key(*id))
            .filter_map(|(_, (row, _))| row.next_visit_at)
            .min();
        match (from_disk, overlay) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Due window from in-memory schedule (0 SQL reads).
    pub(super) fn due_window_in_memory(
        &self,
        domain_key: Option<&str>,
        now_ms: u64,
        limit: usize,
    ) -> Vec<DueEntry> {
        self.whole.as_ref().map_or_else(Vec::new, |whole| {
            whole.due_window(domain_key, now_ms, limit, &self.active)
        })
    }

    /// Merges a reader's due window with the unwritten overlay and excludes
    /// active/in-flight revisits.
    pub(super) fn merge_due_window(
        &self,
        mut from_disk: Vec<DueEntry>,
        domain_key: Option<&str>,
        now_ms: u64,
        limit: usize,
    ) -> Vec<DueEntry> {
        // Drop entries superseded by unwritten overlay or already active.
        from_disk.retain(|entry| {
            !self.active.contains_key(&entry.work_item)
                && !self.unwritten.contains_key(&entry.work_item)
        });
        for (id, (row, _)) in &self.unwritten {
            if self.active.contains_key(id) {
                continue;
            }
            if let Some(domain) = domain_key
                && row.domain_key != domain
            {
                continue;
            }
            if let Some(due) = row.next_visit_at
                && due <= now_ms
            {
                from_disk.push(DueEntry {
                    work_item: id.clone(),
                    domain_key: row.domain_key.clone(),
                    next_visit_at: due,
                });
            }
        }
        from_disk.sort_by(|a, b| {
            a.next_visit_at
                .cmp(&b.next_visit_at)
                .then_with(|| a.work_item.cmp(&b.work_item))
        });
        from_disk.truncate(limit);
        from_disk
    }

    /// Marks `entry` as queued in `admission`'s per-domain queue and returns
    /// its [`Waiting`] item. Uses cached `Leased` template when available.
    pub(super) fn queue_due(&mut self, entry: &DueEntry) -> Waiting {
        let template = self.templates.get(&entry.work_item).cloned();
        let waiting = template.as_ref().map_or_else(
            || Waiting {
                id: entry.work_item.clone(),
                key: entry.domain_key.clone(),
                priority: 0,
                order: entry.work_item.parse::<i64>().unwrap_or(0),
                forefront: false,
            },
            |leased| leased.waiting.clone(),
        );
        self.active.insert(
            entry.work_item.clone(),
            ActiveRevisit {
                leased: template.map(Leased::for_revisit),
                previous_outcome: (OutcomeKind::Succeeded, String::new()),
                in_flight: false,
            },
        );
        waiting
    }

    /// Whether `id` is currently tracked as an active revisit (waiting or in flight).
    pub(super) fn is_active(&self, id: &str) -> bool {
        self.active.contains_key(id)
    }

    /// Takes the cached [`Leased`] row for an active revisit being leased and
    /// marks it in flight, returning `(cached_leased, previous_outcome)`.
    pub(super) fn begin_lease(&mut self, id: &str) -> Option<(Option<Leased>, CountedAs)> {
        let active = self.active.get_mut(id)?;
        active.in_flight = true;
        Some((active.leased.take(), active.previous_outcome.clone()))
    }

    /// Cancels all queued-but-unleased revisits (called on `run.finish()`).
    pub(super) fn drop_unleased(&mut self) {
        self.active.retain(|_, active| active.in_flight);
    }
}
