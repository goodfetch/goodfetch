//! Whether a live crawl still takes new work (ADR 0102; #914, #1005): a
//! crawl about to end closes its intake in a job of its Run state's writer
//! once that job finds no work item pending, and every live write that
//! gives it work, a re-admission or a seed added, checks the intake in its
//! own job of the same writer. The writer runs its jobs one at a time, so
//! a call either commits its work before the crawl's last look, which
//! then finds it pending and goes on delivering, or finds the intake
//! closed and commits nothing: work is never answered and then left
//! pending in a Run that ended.
#![expect(
    clippy::disallowed_types,
    reason = "the flag is read and set only inside the Run state's writer jobs, which that writer runs one after another, so the writer orders every access and there is no interleaving for loom to explore (ADR 0060)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;

use crate::work_items;

/// A live crawl's intake, shared by its delivery and the calls that give
/// it work; each of them reads or changes it only inside a job of the
/// crawl's Run state writer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Intake(Arc<AtomicBool>);

impl Intake {
    /// Closes the intake when the run file `connection` holds no work item
    /// pending or leased, and answers whether it closed: a crawl that
    /// finds work goes on delivering it, its intake still open, and one
    /// with a work item still leased has not settled it.
    ///
    /// # Errors
    ///
    /// The run file's error when its work items could not be read.
    pub(crate) fn close_if_drained(&self, connection: &Connection) -> rusqlite::Result<bool> {
        if work_items::any_unsettled(connection)? {
            return Ok(false);
        }
        self.0.store(true, Ordering::SeqCst);
        Ok(true)
    }

    /// Runs `work`, a write that gives the crawl new work, while the
    /// intake is open, and answers what it returned; `None`, running
    /// nothing, once the intake closed.
    ///
    /// # Errors
    ///
    /// `work`'s error.
    pub(crate) fn admitting<T>(
        &self,
        work: impl FnOnce() -> rusqlite::Result<T>,
    ) -> rusqlite::Result<Option<T>> {
        if self.0.load(Ordering::SeqCst) {
            return Ok(None);
        }
        work().map(Some)
    }
}
