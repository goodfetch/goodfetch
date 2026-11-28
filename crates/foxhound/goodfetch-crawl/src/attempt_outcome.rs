//! An attempt's outcome, one job of the run state's writer (ADR 0138):
//! what its Session's score changed, its tier choice once classified, then
//! its commit, its links, Lease's commit and Records included, or its work
//! item's end. What its page teaches the Frontier is no part of it: the
//! page noted it, and the end of a batch teaches it (#2169). A job's
//! failure rolls back that attempt's outcome alone, every other job of its
//! group left as it was.

use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::Keys;
use goodfetch_engine::egress::Policy;
use goodfetch_frontier::revisit::{self, RevisitRow};
use goodfetch_frontier::sitemap;
use goodfetch_session_pool::Changes;
use rusqlite::Connection;

use crate::lease::{Commit, Committed};
use crate::tier_choice::{self, Unit};
use crate::work_items::{self, Again, AttemptKey, Changed, EndWrite, Leased, Unsuccessful};

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
        ended_at: SystemTime,
    },
    /// Its sitemap or syndication feed's `document` is read, its entries
    /// admitted under `egress` and `invariants` (ADRs 0093 and 0139).
    Source {
        attempt: AttemptKey<String>,
        egress: Arc<Policy>,
        invariants: Arc<goodfetch_frontier::Invariants>,
        document: SourceDocument,
        options: Box<SourceReadOptions>,
        keys: Keys,
        work: CountedWork,
    },
}

/// Options for reading a sitemap or syndication source document (ADR 0139).
#[derive(Clone, Debug, Default)]
pub(crate) struct SourceReadOptions {
    /// Sitemap date/age/change options.
    pub(crate) sitemap: sitemap::ReadOptions,
    /// `<changefreq>` and `<priority>` site-hint switches.
    pub(crate) hints: work_items::SitemapHintPolicy,
    /// When `crawl.continuous` is on, plan for scheduling the source's next
    /// standing re-read in the same writer transaction.
    pub(crate) revisit: Option<SourceRevisitPlan>,
}

/// Parameters for scheduling a standing sitemap source's next re-read in a
/// Continuous crawl (ADR 0139).
#[derive(Clone, Debug)]
pub(crate) struct SourceRevisitPlan {
    /// `crawl.sitemap.revisit` policy (adaptive 24h / 1h..30d by default).
    pub(crate) sitemap_policy: goodfetch_protocol::foxhound::continuous::RevisitPolicy,
    /// `crawl.sitemap.news_revisit` policy (adaptive 1h / 15m..24h by default).
    pub(crate) news_policy: goodfetch_protocol::foxhound::continuous::RevisitPolicy,
    /// Current wall-clock time in milliseconds since the Unix epoch.
    pub(crate) now_ms: u64,
    /// The source's URL.
    pub(crate) url: String,
    /// The source's domain key.
    pub(crate) domain_key: String,
    /// Previous persisted revisit row for this source, if any.
    pub(crate) previous: Option<RevisitRow>,
    /// Whether `run.finish()` has been requested (schedule no further re-read).
    pub(crate) let_finish: bool,
}

/// The parsed document of a non-handler source fetch (sitemap or syndication
/// feed).
pub(crate) enum SourceDocument {
    Sitemap(sitemap::Document),
    Syndication(goodfetch_frontier::syndication::FeedDocument),
}

/// An attempt's outcome, written in one job.
pub(crate) struct Outcome {
    /// What its Session's score changed in the crawl's Session pool.
    pub(crate) scored: Changes,
    /// Its Label's tier choice on its domain, once its sample classified
    /// it.
    pub(crate) chosen: Option<(String, String, Unit)>,
    /// Updated Continuous crawl revisit row written in the same transaction
    /// when the attempt commits (ADR 0139, ADR 0140); `None` on batch crawls.
    pub(crate) revisit: Option<RevisitRow>,
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
    /// its read added, keyed, in order, as its first lease takes it, plus
    /// the updated standing-source [`RevisitRow`] in a Continuous crawl.
    SitemapRead {
        added: Option<Vec<Leased>>,
        revisit: Option<Box<RevisitRow>>,
    },
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
            revisit,
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
                let (attempt, ended_at) = (commit.attempt().owned(), *commit.ended_at());
                match commit.write(connection, &mut changed)? {
                    Some(Ok(committed)) => {
                        if let Some(row) = revisit {
                            revisit::upsert(connection, &row)?;
                        }
                        Ended::Committed(committed)
                    }
                    Some(Err(refusal)) => end(
                        &mut EndWrite {
                            connection,
                            ended_at,
                            changed: &mut changed,
                        },
                        attempt.borrowed(),
                        refused(refusal),
                    )?,
                    None => Ended::LeaseLost,
                }
            }
            Ending::End {
                work_item,
                lease_epoch,
                unsuccessful,
                ended_at,
            } => {
                let ended = end(
                    &mut EndWrite {
                        connection,
                        ended_at,
                        changed: &mut changed,
                    },
                    AttemptKey::new(&work_item, lease_epoch),
                    unsuccessful,
                )?;
                if matches!(ended, Ended::Unsuccessful { ended: true, .. })
                    && let Some(row) = revisit
                {
                    revisit::upsert(connection, &row)?;
                }
                ended
            }
            Ending::Source {
                attempt,
                egress,
                invariants,
                document,
                options,
                keys,
                work,
            } => write_source(
                connection,
                &attempt,
                &egress,
                &invariants,
                &document,
                *options,
                (&keys, &work, &mut changed),
            )?,
        };
        Ok(Written {
            ended,
            changed: changed.with_sitemaps(connection)?,
        })
    }
}

/// Ends `attempt` `unsuccessful` as `write` says, as [`work_items::end`]
/// does.
pub(crate) fn end(
    write: &mut EndWrite<'_>,
    attempt: AttemptKey<&str>,
    unsuccessful: Unsuccessful,
) -> rusqlite::Result<Ended> {
    let (recorded, again) = match &unsuccessful {
        Unsuccessful::Retried(failure) => (Some(failure.code.clone()), Again::Retried),
        Unsuccessful::Failed(failure) => (Some(failure.code.clone()), Again::Not),
        Unsuccessful::Skipped { .. }
        | Unsuccessful::Blocked(_)
        | Unsuccessful::Unchanged
        | Unsuccessful::Gone => (None, Again::Not),
        Unsuccessful::Released => (None, Again::Released),
    };
    let ended = work_items::end(
        write.connection,
        attempt.work_item,
        attempt.lease_epoch,
        unsuccessful,
        write.ended_at,
        write.changed,
    )?;
    Ok(Ended::Unsuccessful {
        ended,
        recorded,
        again,
    })
}

/// Writes a sitemap or syndication source read and, in a Continuous crawl,
/// upserts its next standing-source [`RevisitRow`] in the same transaction.
fn write_source(
    connection: &Connection,
    attempt: &AttemptKey<String>,
    egress: &Policy,
    invariants: &goodfetch_frontier::Invariants,
    document: &SourceDocument,
    options: SourceReadOptions,
    (keys, work, changed): (&Keys, &CountedWork, &mut Changed),
) -> rusqlite::Result<Ended> {
    let (read, source_revisit) = match document {
        SourceDocument::Sitemap(document) => {
            let read = work_items::read_sitemap(
                connection,
                invariants,
                egress,
                (&attempt.work_item, attempt.lease_epoch),
                document,
                (options.sitemap, &options.hints),
                (keys, work, changed),
            )?;
            match read {
                Some((added, outcome)) => {
                    let row = options
                        .revisit
                        .map(|plan| schedule_sitemap_revisit(&attempt.work_item, plan, outcome));
                    if let Some(row) = &row {
                        revisit::upsert(connection, row)?;
                    }
                    (Some(added), row.map(Box::new))
                }
                None => (None, None),
            }
        }
        SourceDocument::Syndication(document) => (
            work_items::read_syndication(
                connection,
                invariants,
                egress,
                (&attempt.work_item, attempt.lease_epoch),
                document,
                (keys, work, changed),
            )?,
            None,
        ),
    };
    Ok(Ended::SitemapRead {
        added: read
            .map(|added| work_items::leasable_added(connection, &added))
            .transpose()?,
        revisit: source_revisit,
    })
}

/// Computes the next standing-source [`RevisitRow`] for a sitemap read in a
/// Continuous crawl (ADR 0139).
fn schedule_sitemap_revisit(
    work_item: &str,
    plan: SourceRevisitPlan,
    outcome: sitemap::ReadOutcome,
) -> RevisitRow {
    let policy = if outcome.is_news {
        &plan.news_policy
    } else {
        &plan.sitemap_policy
    };
    let previous = plan
        .previous
        .as_ref()
        .map_or_else(revisit::PreviousState::default, |row| {
            revisit::PreviousState {
                interval_ms: row.interval_ms,
                visits: row.visits,
                changes: row.changes,
            }
        });
    let visit_result = if outcome.changed {
        revisit::VisitResult::Changed
    } else {
        revisit::VisitResult::Unchanged
    };
    let scheduled = if plan.let_finish {
        None
    } else {
        revisit::next_visit(
            policy,
            previous,
            visit_result,
            revisit::SiteHints::default(),
            plan.now_ms,
        )
    };
    RevisitRow {
        work_item: work_item.to_owned(),
        identity: plan.url,
        domain_key: plan.domain_key,
        next_visit_at: scheduled.map(|s| s.next_visit_at),
        interval_ms: scheduled.map(|s| s.interval_ms),
        last_fetched_at: Some(plan.now_ms),
        last_changed_at: if outcome.changed {
            Some(plan.now_ms)
        } else {
            plan.previous.as_ref().and_then(|r| r.last_changed_at)
        },
        signature: None,
        etag: None,
        last_modified: None,
        visits: previous.visits.saturating_add(1),
        changes: if outcome.changed {
            previous.changes.saturating_add(1)
        } else {
            previous.changes
        },
        consecutive_gone: 0,
        last_tier: Some("source".to_owned()),
        at_min_floor: scheduled.is_some_and(|s| s.clamped_at_min),
    }
}

#[cfg(test)]
mod tests;
