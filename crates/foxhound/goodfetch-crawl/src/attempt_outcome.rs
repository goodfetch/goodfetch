//! An attempt's outcome, one job of the run state's writer (ADR 0138):
//! what its Session's score changed, its tier choice once classified, what
//! its page taught the Frontier, then its commit, its links, Lease's commit
//! and Records included, or its work item's end. Learning sits in a nested
//! savepoint, so its failure loses only what the page would have taught and
//! never fails the outcome, and an attempt that failed still commits what
//! it learned. A job's failure rolls back that attempt's outcome alone,
//! every other job of its group left as it was.

use std::collections::BTreeSet;
use std::sync::Arc;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::{Keys, Waiting};
use goodfetch_engine::egress::Policy;
use goodfetch_fetch_tier::Redirected;
use goodfetch_frontier::{EndMerged, PageSeen, Redirect, Url, sitemap};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::frontier::{HttpBody, HttpMethod};
use goodfetch_protocol::run::PointInTime;
use goodfetch_session_pool::Changes;
use rusqlite::Connection;

use crate::lease::{Commit, Committed};
use crate::tier_choice::{self, Unit};
use crate::work_items::{self, Changed, Unsuccessful};

/// What a fetched page teaches the Frontier (ADRs 0005 and 0120): each
/// redirect hop its chain followed, its page signature and
/// `rel=canonical`, and the pending requests a rule it taught merges,
/// ended `skipped` with `duplicate` at `ended_at`; those a rule it stopped
/// had ended so are pending again.
#[derive(Debug)]
pub(crate) struct Learning {
    pub(crate) request: CrawlRequest,
    /// Each redirect hop its crawl request's chain followed, in order.
    pub(crate) redirects: Arc<[Redirected]>,
    /// Its document, when the page holds one.
    pub(crate) document: Option<Document>,
    pub(crate) ended_at: PointInTime,
    /// What counts the rows of evidence its teaching reads.
    pub(crate) work: CountedWork,
}

/// A fetched page's document, as the Frontier learns from it.
#[derive(Debug)]
pub(crate) struct Document {
    /// The URL it was fetched at.
    pub(crate) url: Url,
    pub(crate) signature: [u8; 32],
    pub(crate) canonical: Option<Url>,
}

impl Learning {
    /// Teaches it in `connection`'s transaction, each redirect hop first
    /// and then the document, answering each pending
    /// request a rule it taught merged, and each one a rule it stopped
    /// gave back, by ID with the domain key it waits under: the requests
    /// whose domain's queue no longer holds what the run file says. With
    /// them, what ending and giving them back changed of the crawl's
    /// requests by state.
    ///
    /// # Errors
    ///
    /// The run state's: the caller's transaction may then hold part of it.
    pub(crate) fn teach(self, connection: &Connection) -> rusqlite::Result<Moved> {
        let Self {
            request,
            redirects,
            document,
            ended_at,
            work,
        } = self;
        let mut merged = Merged {
            ended_at,
            moved: Vec::new(),
            changed: Changed::default(),
        };
        for hop in redirects.iter() {
            // A method the protocol cannot name is no `GET`, which alone
            // is evidence.
            let Ok(method) = HttpMethod::parse(hop.method.as_str()) else {
                continue;
            };
            goodfetch_frontier::learn_redirect(
                connection,
                Redirect {
                    from: &hop.from,
                    to: &hop.to,
                    method: &method,
                    body: hop.body.as_deref(),
                },
                &mut merged,
                &work,
            )?;
        }
        if let Some(Document {
            url,
            signature,
            canonical,
        }) = &document
        {
            goodfetch_frontier::learn(
                connection,
                PageSeen {
                    crawl_request: &request.id,
                    url,
                    method: &request.method,
                    body: request.body.as_ref().map(HttpBody::bytes),
                    signature: *signature,
                    canonical: canonical.as_ref(),
                },
                &mut merged,
                &work,
            )?;
        }
        Ok((merged.moved, merged.changed))
    }

    /// Teaches it as [`Learning::teach`] does, in a savepoint of its own
    /// inside `connection`'s transaction: one that fails is rolled back
    /// alone, and answers no request merged and nothing changed.
    ///
    /// # Errors
    ///
    /// The run state's, when the savepoint could not be rolled back.
    pub(crate) fn teach_apart(self, connection: &Connection) -> rusqlite::Result<Moved> {
        connection
            .prepare_cached("SAVEPOINT learning")?
            .execute([])?;
        match self.teach(connection) {
            Ok(merged) => {
                connection.prepare_cached("RELEASE learning")?.execute([])?;
                Ok(merged)
            }
            Err(error) => {
                tracing::warn!(%error, "the Frontier could not learn from a fetched page");
                connection
                    .prepare_cached("ROLLBACK TO learning")?
                    .execute([])?;
                connection.prepare_cached("RELEASE learning")?.execute([])?;
                Ok(Moved::default())
            }
        }
    }
}

/// What a lesson to the Frontier moved: each pending request a rule it
/// taught merged and each one a rule it stopped gave back, by ID with the
/// domain key it waits under, and what that changed of the crawl's
/// requests by state.
pub(crate) type Moved = (Vec<(String, String)>, Changed);

/// The crawl's side of a rule learned later: the pending requests it
/// merged, ended at `ended_at`, and those its stop made pending again, by
/// ID with their domain key, and what both changed of the crawl's requests
/// by state.
struct Merged {
    ended_at: PointInTime,
    moved: Vec<(String, String)>,
    changed: Changed,
}

impl EndMerged for Merged {
    fn end_merged(
        &mut self,
        connection: &Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<goodfetch_frontier::Ended>> {
        let ended = work_items::end_merged(
            connection,
            crawl_requests,
            &self.ended_at,
            &mut self.changed,
        )?;
        Ok(ended
            .into_iter()
            .map(|(id, url, key)| {
                if let Some(key) = key {
                    self.moved.push((id.clone(), key));
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
        Ok(given
            .into_iter()
            .map(|(id, key)| {
                if let Some(key) = key {
                    self.moved.push((id.clone(), key));
                }
                id
            })
            .collect())
    }
}

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
    /// What its page teaches the Frontier, when its attempt left it to its
    /// outcome.
    pub(crate) learning: Option<Learning>,
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
    /// whether it waits again for a retry.
    Unsuccessful {
        ended: bool,
        recorded: Option<String>,
        retried: bool,
    },
    /// Its sitemap was read, when its lease still held: each work item
    /// its read added, keyed, in order.
    SitemapRead(Option<Vec<Waiting>>),
}

/// What an outcome's job answers.
#[derive(Debug)]
pub(crate) struct Written {
    /// How its work item ended.
    pub(crate) ended: Ended,
    /// Each pending request a rule its page taught merged, and each one a
    /// rule it stopped gave back, by ID with its domain key.
    pub(crate) moved: Vec<(String, String)>,
    /// What the job changed of the crawl's requests by state, for the
    /// crawl's Run state to count.
    pub(crate) changed: Changed,
}

impl Outcome {
    /// Writes it in `connection`'s transaction, answering how its work item
    /// ended, each pending request a rule its page taught merged, by ID
    /// with its domain key, and what it changed of the crawl's requests by
    /// state.
    ///
    /// # Errors
    ///
    /// The run state's: the job's savepoint then rolls it back whole.
    pub(crate) fn write(self, connection: &Connection) -> rusqlite::Result<Written> {
        let Self {
            scored,
            chosen,
            learning,
            ending,
        } = self;
        if !scored.is_empty() {
            scored.write(connection)?;
        }
        if let Some((label, domain_key, unit)) = chosen {
            tier_choice::record(connection, &label, &domain_key, &unit)?;
        }
        let (moved, mut changed) = match learning {
            Some(learning) => learning.teach_apart(connection)?,
            None => Moved::default(),
        };
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
            } => Ended::SitemapRead(work_items::read_sitemap(
                connection,
                &invariants,
                &egress,
                (&work_item, lease_epoch),
                &document,
                (&keys, &work, &mut changed),
            )?),
        };
        Ok(Written {
            ended,
            moved,
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
    let retried = matches!(unsuccessful, Unsuccessful::Retried(_));
    let recorded = match &unsuccessful {
        Unsuccessful::Retried(failure) | Unsuccessful::Failed(failure) => {
            Some(failure.code.clone())
        }
        Unsuccessful::Skipped { .. } | Unsuccessful::Blocked(_) => None,
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
        retried,
    })
}

#[cfg(test)]
mod tests;
