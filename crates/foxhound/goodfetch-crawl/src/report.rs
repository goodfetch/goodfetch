//! What a crawl adds to its Run's report (ADR 0102), through the
//! composition's report part: `requests`, how its requests ended, from each
//! one's latest outcome, with the counts by skip reason, Error code and
//! Block cause, and those left pending; `records`, per Dataset, the Records
//! its succeeded requests committed, dropped and rejected; `learned_rules`,
//! the rules its Frontier learned, each with its evidence and every URL it
//! merged (ADR 0005); and, for a crawl
//! that finished with no seed reaching a handler, `nothingDone`'s
//! `no_seed_handled`, counting its seeds by why each never reached one. A
//! Run of another kind gets none of them.

use std::collections::BTreeMap;

use goodfetch_engine::runs::ReportPart;
use goodfetch_protocol::foxhound::frontier::Enqueued;
use goodfetch_protocol::foxhound::outcomes::{CrawlRequestOutcome, DatasetRecords, Ending};
use goodfetch_protocol::foxhound::report::{CountOf, RequestCounts};
use goodfetch_protocol::run::{Lifecycle, NothingDone, RunKind, RunReport, SeedsKept};
use rusqlite::{Connection, OptionalExtension as _};

/// The crawl's part of its Run's report.
pub(crate) struct CrawlParts;

impl ReportPart for CrawlParts {
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        let kind: String = connection.query_row("SELECT kind FROM run", [], |row| row.get(0))?;
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
        let handled: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM work_items WHERE attempts > 0)",
            [],
            |row| row.get(0),
        )?;
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
    let mut answers = connection.prepare("SELECT answer FROM seeds ORDER BY position")?;
    let mut latest = connection.prepare(
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

/// How the crawl's requests ended, by each one's latest outcome, and its
/// Records per Dataset.
pub(crate) fn ended(
    connection: &Connection,
) -> rusqlite::Result<(RequestCounts, Vec<DatasetRecords>)> {
    let mut counts = RequestCounts {
        pending: crate::work_items::left(connection)?,
        ..RequestCounts::default()
    };
    let (mut skips, mut codes, mut causes) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut records: BTreeMap<String, DatasetRecords> = BTreeMap::new();
    let mut statement = connection.prepare(
        "SELECT outcome FROM outcomes WHERE position IN
             (SELECT MAX(position) FROM outcomes GROUP BY work_item)",
    )?;
    let outcomes = statement.query_map([], |row| {
        let text: String = row.get(0)?;
        serde_json::from_str::<CrawlRequestOutcome>(&text)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    })?;
    for outcome in outcomes {
        match outcome?.ending {
            Ending::Succeeded {
                records: committed, ..
            } => {
                counts.succeeded += 1;
                for dataset in committed {
                    let total =
                        records
                            .entry(dataset.dataset.clone())
                            .or_insert_with(|| DatasetRecords {
                                dataset: dataset.dataset.clone(),
                                committed: 0,
                                dropped: 0,
                                rejected: 0,
                            });
                    total.committed += dataset.committed;
                    total.dropped += dataset.dropped;
                    total.rejected += dataset.rejected;
                }
            }
            Ending::Failed { error } => {
                counts.failed += 1;
                *codes.entry(error.code).or_insert(0) += 1;
            }
            Ending::Skipped { reason, .. } => {
                counts.skipped += 1;
                *skips
                    .entry(name_of(serde_json::to_value(reason)))
                    .or_insert(0) += 1;
            }
            Ending::Blocked { cause } => {
                counts.blocked += 1;
                *causes.entry(cause.as_str().to_owned()).or_insert(0) += 1;
            }
        }
    }
    counts.by_skip_reason = counted(skips);
    counts.by_error_code = counted(codes);
    counts.by_block_cause = counted(causes);
    Ok((counts, records.into_values().collect()))
}

fn counted(counts: BTreeMap<String, u64>) -> Vec<CountOf> {
    counts
        .into_iter()
        .map(|(name, count)| CountOf { name, count })
        .collect()
}
