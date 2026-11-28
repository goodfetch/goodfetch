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
use goodfetch_frontier::lastmod_trust::{self, ClaimKind, TrustThresholds};
use goodfetch_frontier::revisit::{self, RevisitRow};
use goodfetch_frontier::sitemap;
use goodfetch_protocol::foxhound::continuous::{LastmodHint, RevisitPolicy};
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

/// Parameters for evaluating `<lastmod>` and `news:publication_date` claims
/// during a standing sitemap read (ADR 0139, spec #1836).
#[derive(Clone, Debug)]
pub(crate) struct LastmodClaimConfig {
    /// Per-Label settings (for `revisit.policy` and `revisit.site_hints.lastmod`).
    pub(crate) labels: Arc<goodfetch_fetch_tier::LabelSettings>,
    /// Run-level fallback `revisit.policy`.
    pub(crate) run_policy: RevisitPolicy,
    /// Run-level fallback `revisit.site_hints.lastmod`.
    pub(crate) run_hint: LastmodHint,
}

/// Parameters for scheduling a standing sitemap or syndication source's next
/// re-read in a Continuous crawl (ADR 0139).
#[derive(Clone, Debug)]
pub(crate) struct SourceRevisitPlan {
    /// `crawl.sitemap.revisit` policy (adaptive 24h / 1h..30d by default).
    pub(crate) sitemap_policy: RevisitPolicy,
    /// `crawl.sitemap.news_revisit` policy (adaptive 1h / 15m..24h by default).
    pub(crate) news_policy: RevisitPolicy,
    /// `crawl.syndication.revisit` policy (`entry_rate` 15m..24h by default).
    pub(crate) syndication_policy: RevisitPolicy,
    /// HTTP caching header floor switches (`revisit.site_hints.*`).
    pub(crate) site_hints: revisit::SiteHintSwitches,
    /// `revisit.site_hints.ttl` (`true` by default).
    pub(crate) use_ttl: bool,
    /// `revisit.site_hints.sy_update_period` (`true` by default).
    pub(crate) use_sy_update_period: bool,
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
    /// Per-Label policies and `<lastmod>` hint switches for claim evaluation.
    pub(crate) lastmod: Option<Arc<LastmodClaimConfig>>,
}

/// Parameters for judging an open `<lastmod>` / `news:publication_date` claim
/// (or recording a miss) inside the outcome's Run-state batch transaction
/// (ADR 0139, ADR 0140 F2, spec #1836).
#[derive(Clone, Debug)]
pub(crate) struct ClaimJudgement {
    /// URL host name.
    pub(crate) host: String,
    /// URL domain key.
    pub(crate) domain_key: String,
    /// Whether the visit observed a content change (`true`) or was unchanged (`false`).
    pub(crate) changed: bool,
    /// Whether this was a revisit (`visits > 1`), as opposed to a first visit.
    pub(crate) is_revisit: bool,
    /// Configured `crawl.sitemap.lastmod_trust.*` thresholds.
    pub(crate) thresholds: TrustThresholds,
}

/// The parsed document of a non-handler source fetch (sitemap or syndication
/// feed).
pub(crate) enum SourceDocument {
    Sitemap(sitemap::Document),
    Syndication {
        document: goodfetch_frontier::syndication::FeedDocument,
        headers: goodfetch_fetch_tier::SourceHeaders,
    },
    SyndicationUnchanged {
        headers: goodfetch_fetch_tier::SourceHeaders,
    },
}

/// Feeds discovered via `<link rel="alternate">` on a committed page, admitted
/// in the same Run-state batch transaction (`crawl.syndication.discover`,
/// ADR 0139, ADR 0140 F2).
pub(crate) struct DiscoveredFeeds {
    pub(crate) feeds: Vec<String>,
    pub(crate) label: String,
    pub(crate) egress: Arc<Policy>,
    pub(crate) invariants: Arc<goodfetch_frontier::Invariants>,
    pub(crate) keys: Keys,
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
    /// `<lastmod>` / `news:publication_date` claim judgement written in the
    /// same transaction as the outcome (0 extra transactions, ADR 0140 F2).
    pub(crate) claim_judgement: Option<ClaimJudgement>,
    /// RSS/Atom feeds discovered on a committed page (`crawl.syndication.discover`).
    pub(crate) discovered_feeds: Option<DiscoveredFeeds>,
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
    /// the updated standing-source [`RevisitRow`] in a Continuous crawl,
    /// any visited URLs whose revisits were brought forward by trusted
    /// `<lastmod>` or `news:publication_date` claims, and whether the file
    /// was flagged `lastmod_generated`.
    SitemapRead {
        added: Option<Vec<Leased>>,
        revisit: Option<Box<RevisitRow>>,
        brought_forward: Vec<RevisitRow>,
        lastmod_generated: bool,
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
            claim_judgement,
            discovered_feeds,
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
                    Some(Ok(mut committed)) => {
                        if let Some(row) = revisit {
                            write_revisit_and_claim(connection, &row, claim_judgement)?;
                        }
                        if let Some(discovered) = discovered_feeds {
                            let added = work_items::admit_discovered_feeds(
                                connection,
                                &discovered.invariants,
                                &discovered.egress,
                                &discovered.feeds,
                                &discovered.label,
                                (&discovered.keys, &mut changed),
                            )?;
                            committed.keyed.extend(added);
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
                    write_revisit_and_claim(connection, &row, claim_judgement)?;
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

/// Upserts `row`, prunes older outcomes for the URL, stamps `visited.last_seen_at`,
/// and judges any open `<lastmod>` / `news:publication_date` claim in the same
/// Run-state batch transaction (0 extra transactions, ADR 0139 / ADR 0140 F2).
fn write_revisit_and_claim(
    connection: &Connection,
    row: &RevisitRow,
    claim_judgement: Option<ClaimJudgement>,
) -> rusqlite::Result<()> {
    revisit::upsert(connection, row)?;
    work_items::prune_old_outcomes(connection, &row.work_item)?;
    if let Some(seen_at_ms) = row.last_fetched_at {
        goodfetch_frontier::touch_visited(connection, &row.work_item, seen_at_ms)?;
    }
    if let Some(judge) = claim_judgement {
        lastmod_trust::judge_visit(
            connection,
            &row.work_item,
            &judge.host,
            &judge.domain_key,
            judge.changed,
            judge.is_revisit,
            judge.thresholds,
        )?;
    }
    Ok(())
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
    if let Some(forget_ms) = invariants.forget_after_ms()
        && let Some(now_ms) = options.revisit.as_ref().map(|plan| plan.now_ms)
    {
        goodfetch_frontier::stamp_unseen_visited(connection, now_ms)?;
        let stale = goodfetch_frontier::forget_stale(connection, now_ms.saturating_sub(forget_ms))?;
        if !stale.is_empty()
            && let Some(unwritten) = invariants.unwritten()
        {
            let identities: Vec<String> = stale.into_iter().map(|(id, _)| id).collect();
            unwritten.forget(&identities);
        }
    }
    let (read, source_revisit, brought_forward, lastmod_generated) = match document {
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
                    let (brought, stamped) = match options.revisit.as_ref() {
                        Some(plan) if options.sitemap.read_lastmod && !plan.let_finish => {
                            apply_sitemap_lastmod_claims(connection, document, plan)?
                        }
                        _ => (Vec::new(), false),
                    };
                    let row = options
                        .revisit
                        .map(|plan| schedule_sitemap_revisit(&attempt.work_item, plan, outcome));
                    if let Some(row) = &row {
                        revisit::upsert(connection, row)?;
                    }
                    (Some(added), row.map(Box::new), brought, stamped)
                }
                None => (None, None, Vec::new(), false),
            }
        }
        SourceDocument::Syndication { document, headers } => write_syndication_source(
            connection,
            attempt,
            egress,
            invariants,
            document,
            headers,
            options,
            (keys, work, changed),
        )?,
        SourceDocument::SyndicationUnchanged { headers } => {
            if work_items::read_syndication_unchanged(
                connection,
                (&attempt.work_item, attempt.lease_epoch),
                changed,
            )? {
                let row = options.revisit.map(|plan| {
                    schedule_syndication_revisit(&attempt.work_item, plan, None, headers, None)
                });
                if let Some(row) = &row {
                    revisit::upsert(connection, row)?;
                }
                (Some(Vec::new()), row.map(Box::new), Vec::new(), false)
            } else {
                (None, None, Vec::new(), false)
            }
        }
    };
    Ok(Ended::SitemapRead {
        added: read
            .map(|added| work_items::leasable_added(connection, &added))
            .transpose()?,
        revisit: source_revisit,
        brought_forward,
        lastmod_generated,
    })
}

type SourceWriteParts = (
    Option<Vec<goodfetch_engine::admission::Waiting>>,
    Option<Box<RevisitRow>>,
    Vec<RevisitRow>,
    bool,
);

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper splitting syndication handling out of write_source"
)]
fn write_syndication_source(
    connection: &Connection,
    attempt: &AttemptKey<String>,
    egress: &Policy,
    invariants: &goodfetch_frontier::Invariants,
    document: &goodfetch_frontier::syndication::FeedDocument,
    headers: &goodfetch_fetch_tier::SourceHeaders,
    options: SourceReadOptions,
    (keys, work, changed): (&Keys, &CountedWork, &mut Changed),
) -> rusqlite::Result<SourceWriteParts> {
    let read = work_items::read_syndication(
        connection,
        invariants,
        egress,
        (&attempt.work_item, attempt.lease_epoch),
        document,
        options.sitemap,
        (keys, work, changed),
    )?;
    match read {
        Some((added, outcome)) => {
            let brought = match options.revisit.as_ref() {
                Some(plan) if !plan.let_finish => {
                    apply_syndication_update_claims(connection, document, plan)?
                }
                _ => Vec::new(),
            };
            let row = options.revisit.map(|plan| {
                schedule_syndication_revisit(
                    &attempt.work_item,
                    plan,
                    Some(document),
                    headers,
                    Some(outcome),
                )
            });
            if let Some(row) = &row {
                revisit::upsert(connection, row)?;
            }
            Ok((Some(added), row.map(Box::new), brought, false))
        }
        None => Ok((None, None, Vec::new(), false)),
    }
}

/// Evaluates `<lastmod>` and `<news:publication_date>` claims on already-visited
/// URLs in a sitemap urlset, opening claims and bringing trusted revisits
/// forward in the same Run-state batch transaction (ADR 0139, spec #1836).
fn apply_sitemap_lastmod_claims(
    connection: &Connection,
    document: &sitemap::Document,
    plan: &SourceRevisitPlan,
) -> rusqlite::Result<(Vec<RevisitRow>, bool)> {
    let sitemap::Document::Entries { entries, .. } = document else {
        return Ok((Vec::new(), false));
    };
    let Some(lastmod_cfg) = plan.lastmod.as_deref() else {
        return Ok((Vec::new(), false));
    };
    let stamped = lastmod_trust::is_stamped_sitemap(entries);
    let base = goodfetch_frontier::Url::parse(&plan.url).ok();
    let mut brought_forward = Vec::new();
    for entry in entries {
        if entry.news_publication_date.is_none() && (stamped || entry.lastmod.is_none()) {
            continue;
        }
        let resolved = match &base {
            Some(base) => base.join(&entry.loc).ok(),
            None => goodfetch_frontier::Url::parse(&entry.loc).ok(),
        };
        let Some(url) = resolved else {
            continue;
        };
        let Some(mut row) = revisit::get_by_identity(connection, url.as_str())? else {
            continue;
        };
        let Some(last_fetched) = row.last_fetched_at else {
            continue;
        };
        let label = work_items::label_of(connection, &row.work_item)?.unwrap_or_default();
        let policy = lastmod_cfg
            .labels
            .revisit_policy(&label, &lastmod_cfg.run_policy);
        let hint = lastmod_cfg
            .labels
            .revisit_site_hints_lastmod(&label, lastmod_cfg.run_hint);
        let host = url.host_str().unwrap_or("");
        if host.is_empty() {
            continue;
        }
        let bring = if let Some(news_date) = entry.news_publication_date
            && lastmod_trust::is_claim_later(news_date, last_fetched, plan.now_ms)
        {
            lastmod_trust::open_claim(
                connection,
                &row.work_item,
                host,
                &row.domain_key,
                ClaimKind::News,
                plan.now_ms,
            )?;
            true
        } else if !stamped
            && let Some(lastmod_date) = entry.lastmod
            && hint != LastmodHint::Off
            && lastmod_trust::is_claim_later(lastmod_date, last_fetched, plan.now_ms)
        {
            lastmod_trust::open_claim(
                connection,
                &row.work_item,
                host,
                &row.domain_key,
                ClaimKind::Lastmod,
                plan.now_ms,
            )?;
            hint == LastmodHint::Always || lastmod_trust::is_host_trusted(connection, host)?
        } else {
            false
        };
        if bring
            && let Some(due) = lastmod_trust::brought_forward_due(
                plan.now_ms,
                last_fetched,
                row.floor_at_ms,
                policy,
            )
            && row.next_visit_at.is_some_and(|cur| due < cur)
        {
            row.next_visit_at = Some(due);
            revisit::upsert(connection, &row)?;
            brought_forward.push(row);
        }
    }
    Ok((brought_forward, stamped))
}

/// Evaluates feed entry update dates on already-visited URLs in a syndication
/// feed, bringing revisits forward without `<lastmod>` learning while tracking
/// false claims per host (ADR 0139, spec #1836).
fn apply_syndication_update_claims(
    connection: &Connection,
    document: &goodfetch_frontier::syndication::FeedDocument,
    plan: &SourceRevisitPlan,
) -> rusqlite::Result<Vec<RevisitRow>> {
    let Some(lastmod_cfg) = plan.lastmod.as_deref() else {
        return Ok(Vec::new());
    };
    let base = goodfetch_frontier::Url::parse(&plan.url).ok();
    let mut brought_forward = Vec::new();
    for entry in &document.entries {
        let Some(updated) = entry.updated_at else {
            continue;
        };
        let resolved = match &base {
            Some(base) => base.join(&entry.link).ok(),
            None => goodfetch_frontier::Url::parse(&entry.link).ok(),
        };
        let Some(url) = resolved else {
            continue;
        };
        let Some(mut row) = revisit::get_by_identity(connection, url.as_str())? else {
            continue;
        };
        let Some(last_fetched) = row.last_fetched_at else {
            continue;
        };
        if !lastmod_trust::is_claim_later(updated, last_fetched, plan.now_ms) {
            continue;
        }
        let host = url.host_str().unwrap_or("");
        if host.is_empty() {
            continue;
        }
        let label = work_items::label_of(connection, &row.work_item)?.unwrap_or_default();
        let policy = lastmod_cfg
            .labels
            .revisit_policy(&label, &lastmod_cfg.run_policy);
        lastmod_trust::open_claim(
            connection,
            &row.work_item,
            host,
            &row.domain_key,
            ClaimKind::News,
            plan.now_ms,
        )?;
        if let Some(due) =
            lastmod_trust::brought_forward_due(plan.now_ms, last_fetched, row.floor_at_ms, policy)
            && row.next_visit_at.is_some_and(|cur| due < cur)
        {
            row.next_visit_at = Some(due);
            revisit::upsert(connection, &row)?;
            brought_forward.push(row);
        }
    }
    Ok(brought_forward)
}

fn syndication_site_hints(
    plan: &SourceRevisitPlan,
    document: Option<&goodfetch_frontier::syndication::FeedDocument>,
    headers: &goodfetch_fetch_tier::SourceHeaders,
    outcome: Option<goodfetch_frontier::syndication::ReadOutcome>,
    has_validators: bool,
) -> revisit::SiteHints {
    let mut hints = revisit::SiteHints::from_headers(
        headers.cache_control.as_deref(),
        headers.expires.as_deref(),
        headers.retry_after.as_deref(),
        headers.date.as_deref(),
        plan.site_hints,
        plan.now_ms,
    );
    if let Some(doc) = document
        && let Some(feed_floor_ms) = doc.floor_duration_ms(plan.use_ttl, plan.use_sy_update_period)
    {
        let feed_floor_at = plan.now_ms.saturating_add(feed_floor_ms);
        hints.floor_at_ms = Some(
            hints
                .floor_at_ms
                .map_or(feed_floor_at, |cur| cur.max(feed_floor_at)),
        );
    }
    hints.has_validators = has_validators;
    hints.entries_per_week = document.and_then(|doc| {
        if let Some(count) = doc.entries_in_last_week(plan.now_ms) {
            #[expect(
                clippy::cast_precision_loss,
                reason = "entry counts fit well within f64 precision"
            )]
            return Some(count as f64);
        }
        let added = outcome.map_or(0, |o| o.added);
        let prev_fetched = plan.previous.as_ref().and_then(|r| r.last_fetched_at)?;
        let elapsed = plan.now_ms.saturating_sub(prev_fetched);
        if added > 0 && elapsed > 0 {
            #[expect(
                clippy::cast_precision_loss,
                reason = "elapsed duration and entry counts fit within f64 precision"
            )]
            Some(added as f64 * (revisit::ONE_WEEK_MS as f64 / elapsed as f64))
        } else {
            None
        }
    });
    hints
}

/// Computes the next standing-source [`RevisitRow`] for a syndication feed read
/// (or 304 Not Modified) in a Continuous crawl under `crawl.syndication.revisit`
/// (`entry_rate` by default) with HTTP caching headers, `<ttl>`, and
/// `sy:updatePeriod` as floors (ADR 0139).
fn schedule_syndication_revisit(
    work_item: &str,
    plan: SourceRevisitPlan,
    document: Option<&goodfetch_frontier::syndication::FeedDocument>,
    headers: &goodfetch_fetch_tier::SourceHeaders,
    outcome: Option<goodfetch_frontier::syndication::ReadOutcome>,
) -> RevisitRow {
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
    let changed = outcome.is_some_and(|o| o.changed);
    let visit_result = if changed {
        revisit::VisitResult::Changed
    } else {
        revisit::VisitResult::Unchanged
    };
    let is_304 = document.is_none();
    let etag = if is_304 {
        headers
            .etag
            .clone()
            .or_else(|| plan.previous.as_ref().and_then(|r| r.etag.clone()))
    } else {
        headers.etag.clone()
    };
    let last_modified = if is_304 {
        headers
            .last_modified
            .clone()
            .or_else(|| plan.previous.as_ref().and_then(|r| r.last_modified.clone()))
    } else {
        headers.last_modified.clone()
    };
    let hints = syndication_site_hints(
        &plan,
        document,
        headers,
        outcome,
        etag.is_some() || last_modified.is_some(),
    );
    let scheduled = if plan.let_finish {
        None
    } else {
        revisit::next_visit(
            &plan.syndication_policy,
            previous,
            visit_result,
            hints,
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
        last_changed_at: if changed {
            Some(plan.now_ms)
        } else {
            plan.previous.as_ref().and_then(|r| r.last_changed_at)
        },
        signature: None,
        etag,
        last_modified,
        visits: previous.visits.saturating_add(1),
        changes: if changed {
            previous.changes.saturating_add(1)
        } else {
            previous.changes
        },
        consecutive_gone: 0,
        last_tier: Some("source".to_owned()),
        at_min_floor: scheduled.is_some_and(|s| s.clamped_at_min),
        floor_at_ms: hints.floor_at_ms,
    }
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
        floor_at_ms: None,
    }
}

#[cfg(test)]
mod tests;
