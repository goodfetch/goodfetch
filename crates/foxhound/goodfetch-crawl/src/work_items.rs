//! A crawl's work items, Records and outcomes in its run state (ADRs 0003,
//! 0079, 0087, 0102 and 0103): the tables Foxhound's contribution adds to
//! every run file, and the statements that move a work item from pending to
//! leased (committed before its handler request is sent, so no handler runs
//! on a lease the run file does not hold), to succeeded with its Records
//! under its lease, back to pending for a retry, or to failed, skipped or
//! blocked. A work item's every ending writes its outcome in the same
//! commit, in commit order, with the errors of the attempts that failed
//! since its previous ending, each kept as it failed. Each statement runs
//! inside one of the run-state writer's commits.

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{CrawlRequest, PushedRecord, Seed};
use goodfetch_protocol::foxhound::outcomes::{
    AttemptError, CrawlRequestOutcome, DatasetRecords, Ending, NothingCommitted, OutcomeRequest,
    SkipReason,
};
use goodfetch_protocol::run::PointInTime;
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::Value;

/// The tables of a crawl's work items, Records and outcomes.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl",
    statements: &[
        "CREATE TABLE IF NOT EXISTS work_items (
            id TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            url TEXT NOT NULL,
            label TEXT NOT NULL,
            depth INTEGER NOT NULL,
            payload TEXT,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            lease_epoch INTEGER NOT NULL,
            error_code TEXT
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS work_items_by_status ON work_items (status, position)",
        "CREATE TABLE IF NOT EXISTS records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS attempt_errors (
            work_item TEXT NOT NULL,
            attempt INTEGER NOT NULL,
            code TEXT NOT NULL,
            message TEXT NOT NULL,
            PRIMARY KEY (work_item, attempt)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS outcomes_by_work_item ON outcomes (work_item, position)",
    ],
};

const PENDING: &str = "pending";
const LEASED: &str = "leased";
const SUCCEEDED: &str = "succeeded";
const FAILED: &str = "failed";
const SKIPPED: &str = "skipped";
const BLOCKED: &str = "blocked";

/// A work item leased to one attempt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Leased {
    /// The work item as its handler receives it.
    pub(crate) crawl_request: CrawlRequest,
    /// Which attempt this is, from 1.
    pub(crate) attempt: u32,
    /// The lease's epoch.
    pub(crate) lease_epoch: u64,
}

/// What an attempt that committed no Record failed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    /// Its Error code.
    pub(crate) code: String,
    /// What it said.
    pub(crate) message: String,
}

/// How an attempt that committed no Record ends its work item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unsuccessful {
    /// Back to pending, its error kept for the work item's outcome.
    Retried(Failure),
    /// Failed, with no retry left.
    Failed(Failure),
    /// Skipped by its handler, with its message.
    Skipped(String),
    /// Blocked by its site.
    Blocked(BlockCause),
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Adds each seed as a pending work item, in order, at depth 0.
pub(crate) fn insert_seeds(connection: &Connection, seeds: &[Seed]) -> rusqlite::Result<()> {
    let mut insert = connection.prepare(
        "INSERT INTO work_items (id, position, url, label, depth, payload, status, attempts, lease_epoch)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, 'pending', 0, 0)",
    )?;
    for (position, seed) in seeds.iter().enumerate() {
        let position = i64::try_from(position + 1).unwrap_or(i64::MAX);
        insert.execute((
            position.to_string(),
            position,
            &seed.url,
            &seed.label,
            seed.payload.as_ref().map(Value::to_string),
        ))?;
    }
    Ok(())
}

/// Leases up to `count` pending work items, in order: each one's attempts
/// and lease epoch go up by one.
pub(crate) fn lease(connection: &Connection, count: usize) -> rusqlite::Result<Vec<Leased>> {
    let mut statement = connection.prepare(
        "UPDATE work_items SET status = 'leased', attempts = attempts + 1, lease_epoch = lease_epoch + 1
         WHERE id IN (SELECT id FROM work_items WHERE status = 'pending' ORDER BY position LIMIT ?1)
         RETURNING id, url, label, payload, attempts, lease_epoch, position",
    )?;
    let mut leased = statement
        .query_map([i64::try_from(count).unwrap_or(i64::MAX)], |row| {
            let payload: Option<String> = row.get(3)?;
            Ok((
                row.get::<_, i64>(6)?,
                Leased {
                    crawl_request: CrawlRequest {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        label: row.get(2)?,
                        payload: payload.and_then(|payload| serde_json::from_str(&payload).ok()),
                    },
                    attempt: row.get(4)?,
                    lease_epoch: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    leased.sort_by_key(|(position, _)| *position);
    Ok(leased.into_iter().map(|(_, leased)| leased).collect())
}

/// Whether the work item `id` is still leased under `lease_epoch`.
fn still_leased(connection: &Connection, id: &str, lease_epoch: u64) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM work_items WHERE id = ?1 AND status = 'leased' AND lease_epoch = ?2",
            (id, integer(lease_epoch)),
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Commits an attempt's Records, once, marks its work item succeeded and
/// writes its outcome, ended at `ended_at`, when its lease is the one the
/// run file holds; answers whether it did.
pub(crate) fn commit(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    records: &[PushedRecord],
    ended_at: &PointInTime,
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let mut insert = connection.prepare(
        "INSERT INTO records (dataset, number, work_item, data)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM records WHERE dataset = ?1), ?2, ?3)",
    )?;
    let mut counted: Vec<DatasetRecords> = Vec::new();
    for record in records {
        insert.execute((&record.dataset, id, record.data.to_string()))?;
        match counted
            .iter_mut()
            .find(|counted| counted.dataset == record.dataset)
        {
            Some(counted) => counted.committed += 1,
            None => counted.push(DatasetRecords {
                dataset: record.dataset.clone(),
                committed: 1,
                dropped: 0,
                rejected: 0,
            }),
        }
    }
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = NULL WHERE id = ?2",
        (SUCCEEDED, id),
    )?;
    let nothing_committed = counted.is_empty().then_some(NothingCommitted::NonePushed);
    write_outcome(
        connection,
        id,
        ended_at,
        Ending::Succeeded {
            records: counted,
            nothing_committed,
        },
    )?;
    Ok(true)
}

/// Ends an attempt that committed nothing as `unsuccessful` says, ended at
/// `ended_at`, when its lease is the one the run file holds; answers
/// whether it did.
pub(crate) fn end(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    unsuccessful: Unsuccessful,
    ended_at: &PointInTime,
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let (status, error_code) = match &unsuccessful {
        Unsuccessful::Retried(failure) => (PENDING, Some(failure.code.as_str())),
        Unsuccessful::Failed(failure) => (FAILED, Some(failure.code.as_str())),
        Unsuccessful::Skipped(_) => (SKIPPED, None),
        Unsuccessful::Blocked(_) => (BLOCKED, None),
    };
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = ?2 WHERE id = ?3",
        (status, error_code, id),
    )?;
    let ending = match unsuccessful {
        Unsuccessful::Retried(failure) => {
            keep_error(connection, id, &failure)?;
            return Ok(true);
        }
        Unsuccessful::Failed(failure) => Ending::Failed {
            error: keep_error(connection, id, &failure)?,
        },
        Unsuccessful::Skipped(message) => Ending::Skipped {
            reason: SkipReason::Handler,
            message: Some(message),
        },
        Unsuccessful::Blocked(cause) => Ending::Blocked { cause },
    };
    write_outcome(connection, id, ended_at, ending)?;
    Ok(true)
}

/// Keeps the error of the work item `id`'s current attempt for its next
/// outcome.
fn keep_error(
    connection: &Connection,
    id: &str,
    failure: &Failure,
) -> rusqlite::Result<AttemptError> {
    let attempt: u32 = connection.query_row(
        "SELECT attempts FROM work_items WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO attempt_errors (work_item, attempt, code, message) VALUES (?1, ?2, ?3, ?4)",
        (id, attempt, &failure.code, &failure.message),
    )?;
    Ok(AttemptError {
        attempt,
        code: failure.code.clone(),
        message: failure.message.clone(),
    })
}

/// Writes the outcome of the work item `id`, next in commit order, with
/// the errors of its attempts since its previous outcome. Attempts count on
/// across a re-admission, so the attempts of each outcome are the work
/// item's attempts so far.
fn write_outcome(
    connection: &Connection,
    id: &str,
    ended_at: &PointInTime,
    ending: Ending,
) -> rusqlite::Result<()> {
    let (url, label, depth, attempts): (String, String, u32, u32) = connection.query_row(
        "SELECT url, label, depth, attempts FROM work_items WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let errors = connection
        .prepare(
            "SELECT attempt, code, message FROM attempt_errors
             WHERE work_item = ?1
               AND attempt > (SELECT coalesce(max(attempts), 0) FROM outcomes WHERE work_item = ?1)
             ORDER BY attempt",
        )?
        .query_map([id], |row| {
            Ok(AttemptError {
                attempt: row.get(0)?,
                code: row.get(1)?,
                message: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let position: u64 = connection.query_row(
        "SELECT coalesce(max(position), 0) + 1 FROM outcomes",
        [],
        |row| Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default()),
    )?;
    let kind = ending.kind();
    let outcome = CrawlRequestOutcome {
        position,
        request: OutcomeRequest {
            id: id.to_owned(),
            url,
            label: label.clone(),
            depth,
        },
        attempts,
        errors,
        ended_at: ended_at.clone(),
        ending,
    };
    connection.execute(
        "INSERT INTO outcomes (position, work_item, kind, label, attempts, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            integer(position),
            id,
            kind.as_str(),
            label,
            attempts,
            serde_json::to_string(&outcome).expect("invariant: an outcome encodes as JSON"),
        ),
    )?;
    Ok(())
}

/// The statuses of the work items `run.retry_failed` re-admits, which
/// `run.failed_requests` lists: `failed`, and with `blocked`, `blocked`.
pub(crate) fn readmitted(blocked: bool) -> &'static [&'static str] {
    if blocked {
        &[FAILED, BLOCKED]
    } else {
        &[FAILED]
    }
}

/// Re-admits every work item [`readmitted`] names, which ends again with a
/// new outcome, and answers their IDs, in order. `run.retry_failed`'s
/// re-admission, whose operation area 6 adds.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "`run.retry_failed` (#1005) calls it")
)]
pub(crate) fn readmit(connection: &Connection, blocked: bool) -> rusqlite::Result<Vec<String>> {
    let statuses = serde_json::to_string(readmitted(blocked)).expect("invariant: names encode");
    let mut ids = connection
        .prepare(
            "UPDATE work_items SET status = 'pending', error_code = NULL
             WHERE status IN (SELECT value FROM json_each(?1))
             RETURNING id, position",
        )?
        .query_map([statuses], |row| Ok((row.get::<_, i64>(1)?, row.get(0)?)))?
        .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;
    ids.sort();
    Ok(ids.into_iter().map(|(_, id)| id).collect())
}

/// Returns every leased work item to pending without counting its attempt:
/// the program's own stop counts nothing (ADR 0093).
pub(crate) fn release_leases(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE work_items SET status = ?1, attempts = attempts - 1 WHERE status = ?2",
        (PENDING, LEASED),
    )?;
    Ok(())
}

/// Whether any work item is still pending.
pub(crate) fn any_pending(connection: &Connection) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM work_items WHERE status = ?1 LIMIT 1",
            [PENDING],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
