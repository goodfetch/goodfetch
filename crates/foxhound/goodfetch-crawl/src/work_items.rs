//! A crawl's work items and Records in its run state (ADRs 0003, 0087 and
//! 0103): the tables Foxhound's contribution adds to every run file, and the
//! statements that move a work item from pending to leased (committed
//! before its handler request is sent, so no handler runs on a lease the
//! run file does not hold), to succeeded with its Records under its lease,
//! back to pending for a retry, or to failed. Each statement runs inside
//! one of the run-state writer's commits.

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::crawl::{CrawlRequest, PushedRecord, Seed};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::Value;

/// The tables of a crawl's work items and Records.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl",
    statements: &[
        "CREATE TABLE IF NOT EXISTS work_items (
            id TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            url TEXT NOT NULL,
            label TEXT NOT NULL,
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
    ],
};

const PENDING: &str = "pending";
const LEASED: &str = "leased";
const SUCCEEDED: &str = "succeeded";
const FAILED: &str = "failed";

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

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Adds each seed as a pending work item, in order.
pub(crate) fn insert_seeds(connection: &Connection, seeds: &[Seed]) -> rusqlite::Result<()> {
    let mut insert = connection.prepare(
        "INSERT INTO work_items (id, position, url, label, payload, status, attempts, lease_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 0, 0)",
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

/// Commits an attempt's Records, once, and marks its work item succeeded,
/// when its lease is the one the run file holds; answers whether it did.
pub(crate) fn commit(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    records: &[PushedRecord],
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let mut insert = connection.prepare(
        "INSERT INTO records (dataset, number, work_item, data)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM records WHERE dataset = ?1), ?2, ?3)",
    )?;
    for record in records {
        insert.execute((&record.dataset, id, record.data.to_string()))?;
    }
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = NULL WHERE id = ?2",
        (SUCCEEDED, id),
    )?;
    Ok(true)
}

/// Ends an attempt that committed nothing, with the Error code `error_code`:
/// its work item returns to pending when `retry`, and fails otherwise, when
/// its lease is the one the run file holds; answers whether it did.
pub(crate) fn fail(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    error_code: &str,
    retry: bool,
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = ?2 WHERE id = ?3",
        (if retry { PENDING } else { FAILED }, error_code, id),
    )?;
    Ok(true)
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
