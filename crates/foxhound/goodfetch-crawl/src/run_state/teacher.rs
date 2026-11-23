//! Learning written behind, the crawl's side (#2169; ADR 0142's note of
//! 2026-11-10): what the crawl's pages and admissions noted with the
//! Frontier is taught at the end of each batch the Run's writer commits,
//! on the writer's thread, in the batch's transaction. A rule taught there
//! ends each pending request it merges `skipped` with `duplicate`, and
//! makes pending again each one a rule it stopped had ended so; the
//! delivery loop hears of them through [`Taught`], takes them out of their
//! domains' queues, or keeps them pending as their next lease takes them,
//! and fills those queues again. The loop may hear of them before their
//! batch commits: a read that starts once it has sees them.

#![expect(
    clippy::disallowed_types,
    reason = "what a batch's end moved, guarded for one merge or one take, never held with another lock, across an await or while the run file is read, so there is no lock order to test under loom"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::run_state::BatchEnd;
use goodfetch_frontier::EndMerged;
use rusqlite::Connection;
use tokio::sync::{Notify, watch};

use crate::work_items::{self, Changed, Leased};

/// What the Frontier's rules moved of the crawl's pending requests: each
/// one a rule taught merged and each one a rule it stopped gave back, with
/// the commits the writer had acknowledged when its batch's end moved it
/// (#2679), and what that changed of the crawl's requests by state.
pub(crate) type Moved = (Vec<(Move, u64)>, Changed);

/// A pending request a rule moved.
#[derive(Debug)]
pub(crate) enum Move {
    /// A rule taught merged it: its ID, with the domain key it waited
    /// under.
    Merged(String, String),
    /// A rule stopped gave it back: pending again, as its next lease takes
    /// it.
    Given(Box<Leased>),
}

/// What the batches' ends moved and the delivery loop has not taken yet,
/// gathered into one: its size is bounded by the crawl's requests, each
/// of which a rule moves at most once a batch.
#[derive(Debug, Default)]
pub(crate) struct Taught {
    moved: Mutex<Moved>,
    ready: Notify,
}

impl Taught {
    /// Adds `moved`, a batch end's, and wakes the loop, unless it moved
    /// nothing.
    fn add(&self, (ids, changed): Moved) {
        if ids.is_empty() && changed.is_empty() {
            return;
        }
        {
            let mut held = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
            held.0.extend(ids);
            held.1.and(changed);
        }
        self.ready.notify_one();
    }

    /// What the batches' ends moved since the last take, once they moved
    /// anything.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future takes nothing: what was moved waits for the
    /// next take.
    pub(crate) async fn taken(&self) -> Moved {
        loop {
            let moved =
                std::mem::take(&mut *self.moved.lock().unwrap_or_else(PoisonError::into_inner));
            if !moved.0.is_empty() || !moved.1.is_empty() {
                return moved;
            }
            self.ready.notified().await;
        }
    }
}

/// What the crawl's Run writes at the end of each batch: the evidence
/// noted with its Frontier, taught.
#[derive(Debug)]
pub(crate) struct Teacher {
    invariants: Arc<goodfetch_frontier::Invariants>,
    clock: Clock,
    commits: watch::Receiver<u64>,
    work: CountedWork,
    taught: Arc<Taught>,
}

impl Teacher {
    /// Teaches what is noted with `invariants`, a merged request ended at
    /// `clock`'s time, in a batch of the writer whose acknowledged
    /// `commits` it reads, the evidence's rows counted on `work`.
    pub(crate) fn new(
        invariants: Arc<goodfetch_frontier::Invariants>,
        (clock, commits): (Clock, watch::Receiver<u64>),
        work: CountedWork,
    ) -> Self {
        Self {
            invariants,
            clock,
            commits,
            work,
            taught: Arc::default(),
        }
    }

    /// Where the delivery loop hears what the batches' ends moved.
    pub(crate) fn taught(&self) -> Arc<Taught> {
        Arc::clone(&self.taught)
    }

    /// Teaches, in `connection`'s transaction, everything noted, and hands
    /// [`Taught`] what it moved.
    ///
    /// # Errors
    ///
    /// The run state's: the transaction may then hold part of it, and what
    /// it took is no longer noted.
    pub(crate) fn teach(&self, connection: &Connection) -> rusqlite::Result<()> {
        if !self.invariants.waiting() {
            return Ok(());
        }
        let mut merged = Merged {
            ended_at: self.clock.point_in_time(),
            moved: Vec::new(),
            changed: Changed::default(),
        };
        self.invariants
            .teach_behind(connection, &mut merged, &self.work)?;
        let changed = merged.changed.with_sitemaps(connection)?;
        // What it moved is in the commit after those acknowledged now.
        let commits = *self.commits.borrow();
        let moved = merged
            .moved
            .into_iter()
            .map(|moving| (moving, commits))
            .collect();
        self.taught.add((moved, changed));
        Ok(())
    }

    /// Teaches as [`Teacher::teach`] does, in a savepoint of its own inside
    /// `connection`'s transaction: one that fails is rolled back alone,
    /// what it took lost.
    ///
    /// # Errors
    ///
    /// The run state's, when the savepoint could not be rolled back.
    pub(crate) fn teach_apart(&self, connection: &Connection) -> rusqlite::Result<()> {
        if !self.invariants.waiting() {
            return Ok(());
        }
        // Named as the writer's own savepoint for each job, which it nests
        // in: the writer's cache holds these statements already, so the
        // crawl's end prepares none (`F2.prepares`).
        connection.prepare_cached("SAVEPOINT write")?.execute([])?;
        if let Err(error) = self.teach(connection) {
            tracing::warn!(%error, "the Frontier could not learn what the crawl noted");
            connection
                .prepare_cached("ROLLBACK TO write")?
                .execute([])?;
        }
        connection.prepare_cached("RELEASE write")?.execute([])?;
        Ok(())
    }
}

impl BatchEnd for Teacher {
    fn waiting(&self) -> bool {
        self.invariants.waiting()
    }

    fn ending(&self, connection: &Connection) -> rusqlite::Result<()> {
        self.teach(connection)
    }
}

/// The crawl's side of a rule taught: the pending requests it merged,
/// ended at `ended_at`, and those its stop made pending again, and what
/// both changed of the crawl's requests by state.
struct Merged {
    ended_at: SystemTime,
    moved: Vec<Move>,
    changed: Changed,
}

impl EndMerged for Merged {
    fn end_merged(
        &mut self,
        connection: &Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<goodfetch_frontier::Ended>> {
        let ended =
            work_items::end_merged(connection, crawl_requests, self.ended_at, &mut self.changed)?;
        Ok(ended
            .into_iter()
            .map(|(id, url, key)| {
                if let Some(key) = key {
                    self.moved.push(Move::Merged(id.clone(), key));
                }
                goodfetch_frontier::Ended {
                    crawl_request: id,
                    url,
                }
            })
            .collect())
    }

    fn give_back(
        &mut self,
        connection: &Connection,
        crawl_requests: &[String],
    ) -> rusqlite::Result<Vec<String>> {
        let given = work_items::give_back(connection, crawl_requests, &mut self.changed)?;
        self.moved.extend(
            work_items::leasable(connection, &given)?
                .into_iter()
                .map(|leased| Move::Given(Box::new(leased))),
        );
        Ok(given)
    }
}
