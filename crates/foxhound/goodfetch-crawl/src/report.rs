//! What a crawl adds to its Run's report (ADR 0102), through the
//! composition's report part: `requests`, how its requests ended, from each
//! one's latest outcome, with the counts by skip reason, Error code and
//! Block cause, and those left pending; `records`, per Dataset, the Records
//! its succeeded requests committed, dropped and rejected; `learned_rules`,
//! the rules its Frontier learned, each with its evidence and every URL it
//! merged (ADR 0005); `sitemaps`, per sitemap seed, its sitemaps read,
//! those it could not read with their URL and why, and how many of their
//! entries were added and not added, by reason (ADRs 0093 and 0102); and,
//! for a crawl that finished with no seed reaching a handler, `nothingDone`'s
//! `no_seed_handled`, counting its seeds by why each never reached one. A
//! Run of another kind gets none of them.

use std::collections::BTreeMap;

use goodfetch_engine::runs::ReportPart;
use goodfetch_protocol::foxhound::frontier::Enqueued;
use goodfetch_protocol::foxhound::outcomes::{
    CrawlRequestOutcome, DatasetRecords, Ending, OutcomeKind,
};
use goodfetch_protocol::foxhound::report::{CountOf, RequestCounts};
use goodfetch_protocol::run::{Lifecycle, NothingDone, RunKind, RunReport, SeedsKept};
use rusqlite::{Connection, OptionalExtension as _};

/// The crawl's part of its Run's report.
pub(crate) struct CrawlParts;

impl ReportPart for CrawlParts {
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        let kind: String = connection
            .prepare_cached("SELECT kind FROM run")
            .and_then(|mut cached| cached.query_row([], |row| row.get(0)))?;
        if kind != RunKind::Crawl.as_str() {
            return Ok(());
        }
        let (requests, records) = ended(connection)?;
        report.kind_parts.insert(
            "requests".to_owned(),
            serde_json::to_value(requests).expect("invariant: request counts encode"),
        );
        report.kind_parts.insert(
            "records".to_owned(),
            serde_json::to_value(records).expect("invariant: Record counts encode"),
        );
        report.kind_parts.insert(
            "learned_rules".to_owned(),
            serde_json::to_value(goodfetch_frontier::learned_rules(connection)?)
                .expect("invariant: learned rules encode"),
        );
        let sitemaps = goodfetch_frontier::sitemap::reports(connection)?;
        if !sitemaps.is_empty() {
            report.kind_parts.insert(
                "sitemaps".to_owned(),
                serde_json::to_value(sitemaps).expect("invariant: sitemap counts encode"),
            );
        }
        let handled: bool = connection
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM work_items WHERE attempts > 0)")
            .and_then(|mut cached| cached.query_row([], |row| row.get(0)))?;
        if report.nothing_done.is_none() && report.lifecycle == Lifecycle::Finished && !handled {
            report.nothing_done = Some(NothingDone::NoSeedHandled {
                seeds: kept(connection)?,
            });
        }
        Ok(())
    }
}

/// How many seeds each reason kept from a handler, in the order of the
/// reasons: the reason the Frontier did not add one, or how its request
/// ended before its handler ran.
fn kept(connection: &Connection) -> rusqlite::Result<Vec<SeedsKept>> {
    let mut kept: BTreeMap<String, u64> = BTreeMap::new();
    let mut answers = connection.prepare_cached("SELECT answer FROM seeds ORDER BY position")?;
    let mut latest = connection.prepare_cached(
        "SELECT outcome FROM outcomes WHERE work_item = ?1 ORDER BY position DESC LIMIT 1",
    )?;
    for answer in answers.query_map([], |row| row.get::<_, String>(0))? {
        let answer: Enqueued = serde_json::from_str(&answer?).map_err(undecoded)?;
        let reason = match answer {
            Enqueued::NotAdded { reason, .. } => Some(name_of(serde_json::to_value(reason))),
            Enqueued::Added { id, .. } => latest
                .query_row([id], |row| row.get::<_, String>(0))
                .optional()?
                .map(|outcome| serde_json::from_str::<CrawlRequestOutcome>(&outcome))
                .transpose()
                .map_err(undecoded)?
                .map(|outcome| ended_as(&outcome.ending)),
        };
        if let Some(reason) = reason {
            *kept.entry(reason).or_insert(0) += 1;
        }
    }
    Ok(kept
        .into_iter()
        .map(|(reason, count)| SeedsKept { reason, count })
        .collect())
}

/// A run-state value that could not be read as JSON.
fn undecoded(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

/// The wire name of a value encoded as a string.
fn name_of(encoded: serde_json::Result<serde_json::Value>) -> String {
    match encoded {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// What an ending is named by: its skip reason, Error code or Block cause,
/// or `succeeded`.
fn ended_as(ending: &Ending) -> String {
    match ending {
        Ending::Succeeded { .. } => "succeeded".to_owned(),
        Ending::Failed { error } => error.code.clone(),
        Ending::Skipped { reason, .. } => name_of(serde_json::to_value(reason)),
        Ending::Blocked { cause } => cause.as_str().to_owned(),
    }
}

/// How the crawl's requests ended, by each ended one's latest outcome, as
/// the commits of its outcomes and re-admissions counted them, a request
/// pending again counted only pending, and its Records per Dataset: a few
/// rows, whatever the number of requests, so each progress read is cheap.
pub(crate) fn ended(
    connection: &Connection,
) -> rusqlite::Result<(RequestCounts, Vec<DatasetRecords>)> {
    const SUCCEEDED: &str = OutcomeKind::Succeeded.as_str();
    const FAILED: &str = OutcomeKind::Failed.as_str();
    const SKIPPED: &str = OutcomeKind::Skipped.as_str();
    const BLOCKED: &str = OutcomeKind::Blocked.as_str();
    let mut counts = RequestCounts {
        pending: crate::work_items::left(connection)?,
        ..RequestCounts::default()
    };
    let (requests, records) = crate::work_items::ended_counts(connection)?;
    for (kind, name, count) in requests {
        let (total, by) = match kind.as_str() {
            SUCCEEDED => (&mut counts.succeeded, None),
            FAILED => (&mut counts.failed, Some(&mut counts.by_error_code)),
            SKIPPED => (&mut counts.skipped, Some(&mut counts.by_skip_reason)),
            BLOCKED => (&mut counts.blocked, Some(&mut counts.by_block_cause)),
            _ => continue,
        };
        *total += count;
        if let Some(by) = by {
            by.push(CountOf { name, count });
        }
    }
    Ok((counts, records))
}
