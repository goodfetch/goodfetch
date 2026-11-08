//! `run.outcomes` and `run.failed_requests` (ADRs 0031, 0079, 0102 and
//! 0117): a crawl's endings, read from its committed state through the
//! engine's Run reader, never its writer, so a reader that stops reading
//! never slows the crawl, and both work on a Run no program holds.
//! `run.outcomes` answers a page of the outcomes after a position that
//! match its filters, which run here in the engine, and waits for the next
//! commit while the Run is live and nothing new matches.
//! `run.failed_requests` lists the latest outcome of each work item
//! `run.retry_failed` would re-admit.

use goodfetch_engine::run::{run_error, through_owner};
use goodfetch_engine::runs::{RunError, RunReader};
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::outcomes::{
    CrawlRequestOutcome, CrawlRequestOutcomes, FailedRequests, OUTCOMES_PAGE, OutcomesAfter,
    RUN_FAILED_REQUESTS, RUN_OUTCOMES, RunFailedRequestsParams, RunOutcomesParams,
};
use rusqlite::{Connection, OptionalExtension as _};

use crate::work_items;

/// The handlers of `run.outcomes` and `run.failed_requests`.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&RUN_OUTCOMES, outcomes),
        Handler::new(&RUN_FAILED_REQUESTS, failed_requests),
    ]
}

/// Which outcomes a page holds, as JSON arrays of names for SQLite's
/// `json_each`, `None` for every one.
struct Filter {
    kinds: Option<String>,
    labels: Option<String>,
    codes: Option<String>,
}

impl Filter {
    fn of(params: &RunOutcomesParams) -> Self {
        let names = |names: Vec<&str>| {
            serde_json::to_string(&names).expect("invariant: names encode as JSON")
        };
        Self {
            kinds: params
                .kinds
                .as_ref()
                .map(|kinds| names(kinds.iter().map(|kind| kind.as_str()).collect())),
            labels: params
                .labels
                .as_ref()
                .map(|labels| names(labels.iter().map(String::as_str).collect())),
            codes: params
                .codes
                .as_ref()
                .map(|codes| names(codes.iter().map(String::as_str).collect())),
        }
    }
}

/// One page as read: the outcomes after a position that match, in commit
/// order, and the position the next page starts after.
struct Page {
    outcomes: Vec<CrawlRequestOutcome>,
    next: u64,
}

async fn outcomes(
    params: RunOutcomesParams,
    context: CallContext,
) -> Result<CrawlRequestOutcomes, CallError> {
    if let Some(answer) = through_owner(&context, &RUN_OUTCOMES, &params.run_id, &params).await? {
        return Ok(answer);
    }
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    let filter = std::sync::Arc::new(Filter::of(&params));
    let mut after = match params.after {
        None => 0,
        Some(OutcomesAfter::Position(position)) => position,
        Some(OutcomesAfter::Now(_)) => reader.read(last_position).await.map_err(failed)?,
    };
    loop {
        let live = reader.is_live().await;
        let page = read_page(&mut reader, after, &filter)
            .await
            .map_err(failed)?;
        let full = page.outcomes.len() == OUTCOMES_PAGE;
        if !page.outcomes.is_empty() || !live {
            return Ok(CrawlRequestOutcomes {
                outcomes: page.outcomes,
                next: page.next,
                ended: !live && !full,
            });
        }
        after = page.next;
        reader.changed().await;
    }
}

async fn read_page(
    reader: &mut RunReader,
    after: u64,
    filter: &std::sync::Arc<Filter>,
) -> Result<Page, RunError> {
    let filter = std::sync::Arc::clone(filter);
    reader
        .read(move |connection| page(connection, after, &filter))
        .await
}

async fn failed_requests(
    params: RunFailedRequestsParams,
    context: CallContext,
) -> Result<FailedRequests, CallError> {
    if let Some(answer) =
        through_owner(&context, &RUN_FAILED_REQUESTS, &params.run_id, &params).await?
    {
        return Ok(answer);
    }
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    let requests = reader
        .read(move |connection| latest_of_readmitted(connection, params.blocked))
        .await
        .map_err(failed)?;
    Ok(FailedRequests { requests })
}

/// Whether the run file holds a crawl's outcomes: a Run no crawl wrote has
/// none.
fn has_outcomes(connection: &Connection) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'outcomes'")
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some())
}

/// The position of the last outcome committed, 0 before the first.
fn last_position(connection: &Connection) -> rusqlite::Result<u64> {
    if !has_outcomes(connection)? {
        return Ok(0);
    }
    connection
        .prepare_cached("SELECT coalesce(max(position), 0) FROM outcomes")
        .and_then(|mut cached| {
            cached.query_row([], |row| {
                Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default())
            })
        })
}

/// The outcomes after `after` that `filter` takes, at most a page, in
/// commit order, read in one transaction.
fn page(connection: &Connection, after: u64, filter: &Filter) -> rusqlite::Result<Page> {
    if !has_outcomes(connection)? {
        return Ok(Page {
            outcomes: Vec::new(),
            next: after,
        });
    }
    let transaction = connection.unchecked_transaction()?;
    let last = last_position(&transaction)?;
    let outcomes = transaction
        .prepare_cached(
            "SELECT outcome FROM outcomes
             WHERE position > ?1 AND position <= ?2
               AND (?3 IS NULL OR kind IN (SELECT value FROM json_each(?3)))
               AND (?4 IS NULL OR label IN (SELECT value FROM json_each(?4)))
               AND (?6 IS NULL OR EXISTS (
                 SELECT 1 FROM json_each(outcome, '$.errors') AS error
                 WHERE json_extract(error.value, '$.code') IN (SELECT value FROM json_each(?6))
               ))
             ORDER BY position LIMIT ?5",
        )?
        .query_map(
            (
                integer(after),
                integer(last),
                filter.kinds.as_deref(),
                filter.labels.as_deref(),
                integer(u64::try_from(OUTCOMES_PAGE).unwrap_or(u64::MAX)),
                filter.codes.as_deref(),
            ),
            |row| row.get::<_, String>(0),
        )?
        .map(|outcome| outcome.and_then(|outcome| decode(&outcome)))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    transaction.finish()?;
    let next = match outcomes.last() {
        Some(outcome) if outcomes.len() == OUTCOMES_PAGE => outcome.position,
        _ => last.max(after),
    };
    Ok(Page { outcomes, next })
}

/// The latest outcome of each work item `run.retry_failed` re-admits with
/// `blocked`, in commit order.
fn latest_of_readmitted(
    connection: &Connection,
    blocked: bool,
) -> rusqlite::Result<Vec<CrawlRequestOutcome>> {
    if !has_outcomes(connection)? {
        return Ok(Vec::new());
    }
    let statuses = serde_json::to_string(work_items::readmitted(blocked))
        .expect("invariant: names encode as JSON");
    connection
        .prepare_cached(
            "SELECT outcomes.outcome FROM outcomes
             JOIN work_items ON work_items.id = outcomes.work_item
             WHERE work_items.status IN (SELECT value FROM json_each(?1))
               AND outcomes.position = (
                 SELECT max(position) FROM outcomes AS later WHERE later.work_item = work_items.id
               )
             ORDER BY outcomes.position",
        )?
        .query_map([statuses], |row| row.get::<_, String>(0))?
        .map(|outcome| outcome.and_then(|outcome| decode(&outcome)))
        .collect()
}

fn decode(outcome: &str) -> rusqlite::Result<CrawlRequestOutcome> {
    serde_json::from_str(outcome).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
