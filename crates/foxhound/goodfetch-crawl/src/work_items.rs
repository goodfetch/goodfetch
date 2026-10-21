//! A crawl's work items and Records in its run state (ADRs 0003, 0004, 0087
//! and 0103): the tables Foxhound's contribution adds to every run file, the
//! domain key and priority each pending work item waits under, which its
//! per-domain queue is filled from again on a resume, and the statements
//! that move a work item from pending to leased (committed
//! before its handler request is sent, so no handler runs on a lease the
//! run file does not hold), to succeeded with its Records under its lease,
//! back to pending for a retry, or to failed. Each statement runs inside
//! one of the run-state writer's commits.

use goodfetch_engine::admission::Waiting;
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
            error_code TEXT,
            domain_key TEXT,
            priority INTEGER NOT NULL
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS work_items_by_status ON work_items (status, position)",
        "CREATE INDEX IF NOT EXISTS work_items_by_domain
            ON work_items (status, domain_key, priority DESC, position)",
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
    /// Its domain key and priority, under which it waits again on a retry.
    pub(crate) waiting: Waiting,
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Adds each seed as a pending work item, in order.
pub(crate) fn insert_seeds(connection: &Connection, seeds: &[Seed]) -> rusqlite::Result<()> {
    let mut insert = connection.prepare(
        "INSERT INTO work_items (id, position, url, label, payload, status, attempts, lease_epoch, priority)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 0, 0, ?6)",
    )?;
    for (position, seed) in seeds.iter().enumerate() {
        let position = i64::try_from(position + 1).unwrap_or(i64::MAX);
        insert.execute((
            position.to_string(),
            position,
            &seed.url,
            &seed.label,
            seed.payload.as_ref().map(Value::to_string),
            seed.priority,
        ))?;
    }
    Ok(())
}

/// Each work item not keyed yet, with its URL.
pub(crate) fn unkeyed(connection: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    connection
        .prepare("SELECT id, url FROM work_items WHERE domain_key IS NULL ORDER BY position")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// Records each work item's domain key.
pub(crate) fn set_keys(connection: &Connection, keys: &[(String, String)]) -> rusqlite::Result<()> {
    let mut update = connection.prepare("UPDATE work_items SET domain_key = ?2 WHERE id = ?1")?;
    for (id, key) in keys {
        update.execute((id, key))?;
    }
    Ok(())
}

/// The pending work items of `key`, or of every key when `None`, at most
/// `bound` of each key, each key's highest priority first and then in
/// order: what fills its per-domain queue.
pub(crate) fn waiting(
    connection: &Connection,
    key: Option<&str>,
    bound: usize,
) -> rusqlite::Result<Vec<Waiting>> {
    let bound = i64::try_from(bound).unwrap_or(i64::MAX);
    let row = |row: &rusqlite::Row<'_>| {
        Ok(Waiting {
            id: row.get(0)?,
            key: row.get(1)?,
            priority: row.get(2)?,
            order: row.get(3)?,
        })
    };
    match key {
        Some(key) => connection
            .prepare(
                "SELECT id, domain_key, priority, position FROM work_items
                 WHERE status = 'pending' AND domain_key = ?1
                 ORDER BY priority DESC, position LIMIT ?2",
            )?
            .query_map((key, bound), row)?
            .collect(),
        None => connection
            .prepare(
                "SELECT id, domain_key, priority, position FROM (
                     SELECT id, domain_key, priority, position, row_number() OVER (
                         PARTITION BY domain_key ORDER BY priority DESC, position
                     ) AS rank
                     FROM work_items WHERE status = 'pending' AND domain_key IS NOT NULL
                 ) WHERE rank <= ?1 ORDER BY position",
            )?
            .query_map([bound], row)?
            .collect(),
    }
}

/// Leases each pending work item of `ids`, in that order: each one's
/// attempts and lease epoch go up by one.
pub(crate) fn lease(connection: &Connection, ids: &[String]) -> rusqlite::Result<Vec<Leased>> {
    let mut statement = connection.prepare(
        "UPDATE work_items SET status = 'leased', attempts = attempts + 1, lease_epoch = lease_epoch + 1
         WHERE id = ?1 AND status = 'pending'
         RETURNING id, url, label, payload, attempts, lease_epoch, domain_key, priority, position",
    )?;
    let mut leased = Vec::with_capacity(ids.len());
    for id in ids {
        let row = statement
            .query_row([id], |row| {
                let payload: Option<String> = row.get(3)?;
                Ok(Leased {
                    crawl_request: CrawlRequest {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        label: row.get(2)?,
                        payload: payload.and_then(|payload| serde_json::from_str(&payload).ok()),
                    },
                    attempt: row.get(4)?,
                    lease_epoch: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                    waiting: Waiting {
                        id: row.get(0)?,
                        key: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        priority: row.get(7)?,
                        order: row.get(8)?,
                    },
                })
            })
            .optional()?;
        leased.extend(row);
    }
    Ok(leased)
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
