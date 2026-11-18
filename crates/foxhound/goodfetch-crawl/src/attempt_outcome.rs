//! An attempt's outcome, one job of the run state's writer (ADR 0138):
//! what its Session's score changed, its tier choice once classified, then
//! its commit, its links, Lease's commit and Records included, or its work
//! item's end. What its page teaches the Frontier is no part of it: the
//! page noted it, and the end of a batch teaches it (#2169). A job's
//! failure rolls back that attempt's outcome alone, every other job of its
//! group left as it was.

use std::sync::Arc;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::Keys;
use goodfetch_engine::egress::Policy;
use goodfetch_frontier::sitemap;
use goodfetch_protocol::run::PointInTime;
use goodfetch_session_pool::Changes;
use rusqlite::Connection;

use crate::lease::{Commit, Committed};
use crate::tier_choice::{self, Unit};
use crate::work_items::{self, Again, Changed, Leased, Unsuccessful};

/// How a Record its Datasets refuse ends the attempt instead: the
/// unsuccessful ending of its work item.
pub(crate) type Refusing = Box<dyn FnOnce(CallError) -> Unsuccessful + Send>;

/// How an attempt's work item ends.
pub(crate) enum Ending {
    /// It commits, or ends as `refused` says when a Record does not hold.
    Commit { commit: Commit, refused: Refusing },
    /// It ends `unsuccessful`, at `ended_at`.
    End {
        work_item: String,
        lease_epoch: u64,
        unsuccessful: Unsuccessful,
        ended_at: PointInTime,
    },
    /// Its sitemap's `document` is read, its entries admitted under the
    /// Run's domain policy `egress` and within its `invariants` (ADR 0093),
    /// each keyed under `keys` as it is added.
    Sitemap {
        work_item: String,
        lease_epoch: u64,
        egress: Arc<Policy>,
        invariants: Arc<goodfetch_frontier::Invariants>,
        document: sitemap::Document,
        keys: Keys,
        work: CountedWork,
    },
}

/// An attempt's outcome, written in one job.
pub(crate) struct Outcome {
    /// What its Session's score changed in the crawl's Session pool.
    pub(crate) scored: Changes,
    /// Its Label's tier choice on its domain, once its sample classified
    /// it.
    pub(crate) chosen: Option<(String, String, Unit)>,
    pub(crate) ending: Ending,
}

/// What an outcome's job did to its work item.
#[derive(Debug)]
pub(crate) enum Ended {
    /// It committed.
    Committed(Committed),
    /// The run file holds a later lease of it: nothing ended it.
    LeaseLost,
    /// It ended unsuccessful, `ended` when the run file still held its
    /// lease, with the failure's code its Run's outcomes count, and
    /// whether it waits again, for a retry or released.
    Unsuccessful {
        ended: bool,
        recorded: Option<String>,
        again: Again,
    },
    /// Its sitemap was read, when its lease still held: each work item
    /// its read added, keyed, in order, as its first lease takes it.
    SitemapRead(Option<Vec<Leased>>),
}

/// What an outcome's job answers.
#[derive(Debug)]
pub(crate) struct Written {
    /// How its work item ended.
    pub(crate) ended: Ended,
    /// What the job changed of the crawl's requests by state, for the
    /// crawl's Run state to count.
    pub(crate) changed: Changed,
}

impl Outcome {
    /// Writes it in `connection`'s transaction, answering how its work item
    /// ended and what it changed of the crawl's requests by state.
    ///
    /// # Errors
    ///
    /// The run state's: the job's savepoint then rolls it back whole.
    pub(crate) fn write(self, connection: &Connection) -> rusqlite::Result<Written> {
        let Self {
            scored,
            chosen,
            ending,
        } = self;
        if !scored.is_empty() {
            scored.write(connection)?;
        }
        if let Some((label, domain_key, unit)) = chosen {
            tier_choice::record(connection, &label, &domain_key, &unit)?;
        }
        let mut changed = Changed::default();
        let ended = match ending {
            Ending::Commit { commit, refused } => {
                let (work_item, lease_epoch) = commit.attempt();
                let (work_item, ended_at) = (work_item.to_owned(), commit.ended_at().clone());
                match commit.write(connection, &mut changed)? {
                    Some(Ok(committed)) => Ended::Committed(committed),
                    Some(Err(refusal)) => end(
                        connection,
                        (&work_item, lease_epoch),
                        refused(refusal),
                        &ended_at,
                        &mut changed,
                    )?,
                    None => Ended::LeaseLost,
                }
            }
            Ending::End {
                work_item,
                lease_epoch,
                unsuccessful,
                ended_at,
            } => end(
                connection,
                (&work_item, lease_epoch),
                unsuccessful,
                &ended_at,
                &mut changed,
            )?,
            Ending::Sitemap {
                work_item,
                lease_epoch,
                egress,
                invariants,
                document,
                keys,
                work,
            } => {
                let read = work_items::read_sitemap(
                    connection,
                    &invariants,
                    &egress,
                    (&work_item, lease_epoch),
                    &document,
                    (&keys, &work, &mut changed),
                )?;
                Ended::SitemapRead(
                    read.map(|added| work_items::leasable_added(connection, &added))
                        .transpose()?,
                )
            }
        };
        Ok(Written {
            ended,
            changed: changed.with_sitemaps(connection)?,
        })
    }
}

/// Ends the attempt of `work_item` under `lease_epoch` `unsuccessful` at
/// `ended_at`, as [`work_items::end`] does, its ending counted in
/// `changed`.
pub(crate) fn end(
    connection: &Connection,
    (work_item, lease_epoch): (&str, u64),
    unsuccessful: Unsuccessful,
    ended_at: &PointInTime,
    changed: &mut Changed,
) -> rusqlite::Result<Ended> {
    let (recorded, again) = match &unsuccessful {
        Unsuccessful::Retried(failure) => (Some(failure.code.clone()), Again::Retried),
        Unsuccessful::Failed(failure) => (Some(failure.code.clone()), Again::Not),
        Unsuccessful::Skipped { .. } | Unsuccessful::Blocked(_) => (None, Again::Not),
        Unsuccessful::Released => (None, Again::Released),
    };
    let ended = work_items::end(
        connection,
        work_item,
        lease_epoch,
        unsuccessful,
        ended_at,
        changed,
    )?;
    Ok(Ended::Unsuccessful {
        ended,
        recorded,
        again,
    })
}

#[cfg(test)]
mod tests;
