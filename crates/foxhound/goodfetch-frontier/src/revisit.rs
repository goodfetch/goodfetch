//! Revisit scheduling and run-file persistence for Continuous crawls (ADR 0139).
//!
//! The revisit schedule is a pure function of the Label's [`RevisitPolicy`],
//! the URL's previous [`RevisitState`], how the visit ended ([`VisitResult`]),
//! the site's caching hints ([`SiteHints`]), and the current wall-clock time
//! in milliseconds since the Unix epoch.
//!
//! Persisted revisit state lives in the run file's `revisits` table, indexed
//! by `(domain_key, next_visit_at, work_item)` so due-window reads run through
//! a covering index. Only a Continuous crawl writes rows here; a batch crawl
//! writes nothing and runs no query against it.

use std::collections::BTreeMap;

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::continuous::RevisitPolicy;
use rusqlite::{Connection, OptionalExtension as _, params};

/// Default bounded window of due revisit rows read per domain queue fill
/// when revisit state is read from the run file (ADR 0139).
pub const DUE_WINDOW_LIMIT: usize = 64;

/// The `revisits` table and its covering due-order indexes in every run file
/// (ADR 0139). Only a Continuous crawl writes rows into it.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-revisits",
    statements: &[
        "CREATE TABLE IF NOT EXISTS revisits (
            work_item TEXT NOT NULL PRIMARY KEY,
            identity TEXT NOT NULL,
            domain_key TEXT NOT NULL,
            next_visit_at INTEGER,
            interval_ms INTEGER,
            last_fetched_at INTEGER,
            last_changed_at INTEGER,
            signature TEXT,
            etag TEXT,
            last_modified TEXT,
            visits INTEGER NOT NULL,
            changes INTEGER NOT NULL,
            consecutive_gone INTEGER NOT NULL DEFAULT 0,
            last_tier TEXT
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS revisits_due
            ON revisits (domain_key, next_visit_at, work_item)
            WHERE next_visit_at IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS revisits_next_due
            ON revisits (next_visit_at, domain_key, work_item)
            WHERE next_visit_at IS NOT NULL",
    ],
};

/// One URL's persisted revisit state in a Continuous crawl (ADR 0139).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RevisitRow {
    /// The crawl request's work item ID, preserved across revisits.
    pub work_item: String,
    /// The URL identity key (normalized URL).
    pub identity: String,
    /// The domain key under which the URL waits in its per-domain queue.
    pub domain_key: String,
    /// Milliseconds since the Unix epoch when the URL is next due, or `None`
    /// when it will not be revisited (`never` or `gone`).
    pub next_visit_at: Option<u64>,
    /// Current revisit interval in milliseconds, if scheduled.
    pub interval_ms: Option<u64>,
    /// Milliseconds since the Unix epoch of the latest completed fetch.
    pub last_fetched_at: Option<u64>,
    /// Milliseconds since the Unix epoch of the latest visit that saw a change.
    pub last_changed_at: Option<u64>,
    /// Page signature digest from the latest fetch.
    pub signature: Option<String>,
    /// Stored `ETag` validator from the latest fetch.
    pub etag: Option<String>,
    /// Stored `Last-Modified` validator from the latest fetch.
    pub last_modified: Option<String>,
    /// Total completed visits (succeeded or unchanged).
    pub visits: u64,
    /// Total visits that observed a change.
    pub changes: u64,
    /// Consecutive 404/410 revisit responses.
    pub consecutive_gone: u32,
    /// Fetch tier that last served the URL.
    pub last_tier: Option<String>,
}

/// Previous schedule-relevant state of a URL before computing its next due date.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviousState {
    /// Current interval in milliseconds from the previous visit, if any.
    pub interval_ms: Option<u64>,
    /// Total completed visits prior to this one.
    pub visits: u64,
    /// Total changes observed prior to this one.
    pub changes: u64,
}

/// How a visit ended for revisit scheduling purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisitResult {
    /// First visit or revisit whose content changed.
    Changed,
    /// Revisit whose content was unchanged (304 or matching signature).
    Unchanged,
}

/// Caching and freshness hints returned by the site (ADR 0139).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteHints {
    /// Earliest wall-clock timestamp (ms since Unix epoch) allowed by site
    /// caching headers (`Cache-Control: max-age`, `Expires`, `Retry-After`).
    pub floor_at_ms: Option<u64>,
}

/// Output of the pure schedule function: the chosen interval and next due time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheduled {
    /// Interval in milliseconds chosen by the policy (before any floor push).
    pub interval_ms: u64,
    /// Wall-clock timestamp (ms since Unix epoch) when the URL is next due.
    pub next_visit_at: u64,
}

/// Pure function computing when a URL is next due for a revisit (ADR 0139).
///
/// Returns `None` when the policy is `never`. Otherwise returns the interval
/// and next due timestamp in milliseconds since the Unix epoch, never earlier
/// than any site floor in `hints`.
#[must_use]
pub fn next_visit(
    policy: &RevisitPolicy,
    _previous: PreviousState,
    _result: VisitResult,
    hints: SiteHints,
    now_ms: u64,
) -> Option<Scheduled> {
    match policy {
        RevisitPolicy::Never => None,
        RevisitPolicy::Every { interval } => {
            let interval_ms = (*interval).max(1);
            let policy_due = now_ms.saturating_add(interval_ms);
            let next_visit_at = hints
                .floor_at_ms
                .map_or(policy_due, |floor| policy_due.max(floor));
            Some(Scheduled {
                interval_ms,
                next_visit_at,
            })
        }
    }
}

/// A due revisit entry read from the `revisits` table's covering index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueEntry {
    /// Work item ID.
    pub work_item: String,
    /// Domain key.
    pub domain_key: String,
    /// Scheduled due timestamp in milliseconds since Unix epoch.
    pub next_visit_at: u64,
}

/// Inserts or updates a URL's revisit row in `connection` inside the caller's
/// active transaction (0 extra transactions, prepared statement cached).
///
/// # Errors
///
/// Returns SQLite errors from preparing or executing the upsert statement.
pub fn upsert(connection: &Connection, row: &RevisitRow) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(
        "INSERT INTO revisits (
             work_item, identity, domain_key, next_visit_at, interval_ms,
             last_fetched_at, last_changed_at, signature, etag, last_modified,
             visits, changes, consecutive_gone, last_tier
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT (work_item) DO UPDATE SET
             identity = excluded.identity,
             domain_key = excluded.domain_key,
             next_visit_at = excluded.next_visit_at,
             interval_ms = excluded.interval_ms,
             last_fetched_at = excluded.last_fetched_at,
             last_changed_at = excluded.last_changed_at,
             signature = excluded.signature,
             etag = excluded.etag,
             last_modified = excluded.last_modified,
             visits = excluded.visits,
             changes = excluded.changes,
             consecutive_gone = excluded.consecutive_gone,
             last_tier = excluded.last_tier",
    )?;
    statement.execute(params![
        row.work_item,
        row.identity,
        row.domain_key,
        row.next_visit_at.map(u64_to_i64),
        row.interval_ms.map(u64_to_i64),
        row.last_fetched_at.map(u64_to_i64),
        row.last_changed_at.map(u64_to_i64),
        row.signature,
        row.etag,
        row.last_modified,
        u64_to_i64(row.visits),
        u64_to_i64(row.changes),
        i64::from(row.consecutive_gone),
        row.last_tier,
    ])?;
    Ok(())
}

/// Reads one URL's revisit row by `work_item`, if present.
///
/// # Errors
///
/// Returns SQLite errors from reading the row.
pub fn get(connection: &Connection, work_item: &str) -> rusqlite::Result<Option<RevisitRow>> {
    let mut statement = connection.prepare_cached(
        "SELECT work_item, identity, domain_key, next_visit_at, interval_ms,
                last_fetched_at, last_changed_at, signature, etag, last_modified,
                visits, changes, consecutive_gone, last_tier
         FROM revisits WHERE work_item = ?1",
    )?;
    statement.query_row([work_item], decode_row).optional()
}

/// Reads all revisit rows from `connection` on Run open/resume so the Run state
/// can hold the revisit schedule in memory while within its budget.
///
/// # Errors
///
/// Returns SQLite errors from reading the table.
pub fn read_all(connection: &Connection) -> rusqlite::Result<Vec<RevisitRow>> {
    let mut statement = connection.prepare_cached(
        "SELECT work_item, identity, domain_key, next_visit_at, interval_ms,
                last_fetched_at, last_changed_at, signature, etag, last_modified,
                visits, changes, consecutive_gone, last_tier
         FROM revisits ORDER BY work_item",
    )?;
    let rows = statement.query_map([], decode_row)?;
    rows.collect()
}

/// Reads up to `limit` due revisit entries for `domain_key` (or globally when
/// `None`) whose `next_visit_at <= now_ms`, through the covering index.
///
/// # Errors
///
/// Returns SQLite errors from reading the index.
pub fn due_window(
    connection: &Connection,
    domain_key: Option<&str>,
    now_ms: u64,
    limit: usize,
) -> rusqlite::Result<Vec<DueEntry>> {
    let now = u64_to_i64(now_ms);
    let cap = i64::try_from(limit).unwrap_or(i64::MAX);
    if let Some(domain) = domain_key {
        let mut statement = connection.prepare_cached(
            "SELECT work_item, domain_key, next_visit_at
             FROM revisits
             WHERE domain_key = ?1 AND next_visit_at IS NOT NULL AND next_visit_at <= ?2
             ORDER BY next_visit_at ASC, work_item ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![domain, now, cap], decode_due)?;
        rows.collect()
    } else {
        let mut statement = connection.prepare_cached(
            "SELECT work_item, domain_key, next_visit_at
             FROM revisits
             WHERE next_visit_at IS NOT NULL AND next_visit_at <= ?1
             ORDER BY next_visit_at ASC, domain_key ASC, work_item ASC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![now, cap], decode_due)?;
        rows.collect()
    }
}

/// Returns the earliest `next_visit_at` across all scheduled revisits in
/// `connection`, using the `revisits_next_due` index, or `None` if none is
/// scheduled.
///
/// # Errors
///
/// Returns SQLite errors from reading the index.
pub fn earliest_due(connection: &Connection) -> rusqlite::Result<Option<u64>> {
    let mut statement = connection.prepare_cached(
        "SELECT next_visit_at FROM revisits
         WHERE next_visit_at IS NOT NULL
         ORDER BY next_visit_at ASC
         LIMIT 1",
    )?;
    statement
        .query_row([], |row| Ok(i64_to_u64(row.get::<_, i64>(0)?)))
        .optional()
}

/// Counts failed work items that have NEVER been visited (`visits == 0` or no
/// row in `revisits`), grouped by error code, for Continuous crawl Completeness
/// (ADR 0139): a later failed revisit does not take back coverage of a URL that
/// was already visited at least once.
///
/// # Errors
///
/// Returns SQLite errors from reading the tables.
pub fn never_visited_failed_by_error_code(
    connection: &Connection,
) -> rusqlite::Result<BTreeMap<String, u64>> {
    let mut statement = connection.prepare_cached(
        "SELECT w.error_code, COUNT(*)
         FROM work_items w
         LEFT JOIN revisits r ON r.work_item = w.id
         WHERE w.status = 'failed'
           AND w.error_code IS NOT NULL
           AND (r.visits IS NULL OR r.visits = 0)
         GROUP BY w.error_code",
    )?;
    let mut counts = BTreeMap::new();
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, i64_to_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (code, count) = row?;
        counts.insert(code, count);
    }
    Ok(counts)
}

/// Whether the run file holds any row in `revisits` (fast O(1) probe).
///
/// # Errors
///
/// Returns SQLite errors from probing the table.
pub fn has_any(connection: &Connection) -> rusqlite::Result<bool> {
    let has_table: bool = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'revisits'")
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some();
    if !has_table {
        return Ok(false);
    }
    connection
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM revisits LIMIT 1)")
        .and_then(|mut cached| cached.query_row([], |row| row.get(0)))
}

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RevisitRow> {
    Ok(RevisitRow {
        work_item: row.get(0)?,
        identity: row.get(1)?,
        domain_key: row.get(2)?,
        next_visit_at: row.get::<_, Option<i64>>(3)?.map(i64_to_u64),
        interval_ms: row.get::<_, Option<i64>>(4)?.map(i64_to_u64),
        last_fetched_at: row.get::<_, Option<i64>>(5)?.map(i64_to_u64),
        last_changed_at: row.get::<_, Option<i64>>(6)?.map(i64_to_u64),
        signature: row.get(7)?,
        etag: row.get(8)?,
        last_modified: row.get(9)?,
        visits: i64_to_u64(row.get::<_, i64>(10)?),
        changes: i64_to_u64(row.get::<_, i64>(11)?),
        consecutive_gone: u32::try_from(row.get::<_, i64>(12)?).unwrap_or(u32::MAX),
        last_tier: row.get(13)?,
    })
}

fn decode_due(row: &rusqlite::Row<'_>) -> rusqlite::Result<DueEntry> {
    Ok(DueEntry {
        work_item: row.get(0)?,
        domain_key: row.get(1)?,
        next_visit_at: i64_to_u64(row.get::<_, i64>(2)?),
    })
}

fn u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[cfg(test)]
mod tests;
