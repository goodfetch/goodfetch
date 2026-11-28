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
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::retry_after::{self, RetryAfter};
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
            last_tier TEXT,
            at_min_floor INTEGER NOT NULL DEFAULT 0
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
    /// Whether the URL is changing faster than its policy's `min` interval allows
    /// and was clamped to `min` (ADR 0139).
    pub at_min_floor: bool,
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

/// Per-header switches controlling which HTTP caching headers act as revisit
/// floors (`revisit.site_hints.*`, ADR 0139). All three are `true` by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SiteHintSwitches {
    /// Respect `Cache-Control: max-age` as a revisit floor.
    pub cache_control: bool,
    /// Respect `Expires` as a revisit floor.
    pub expires: bool,
    /// Respect `Retry-After` as a revisit floor.
    pub retry_after: bool,
}

impl Default for SiteHintSwitches {
    fn default() -> Self {
        Self {
            cache_control: true,
            expires: true,
            retry_after: true,
        }
    }
}

/// Caching and freshness hints returned by the site (ADR 0139).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteHints {
    /// Earliest wall-clock timestamp (ms since Unix epoch) allowed by site
    /// caching headers (`Cache-Control: max-age`, `Expires`, `Retry-After`).
    pub floor_at_ms: Option<u64>,
}

impl SiteHints {
    /// Computes the revisit floor timestamp from HTTP response headers under
    /// `switches`, measured at wall-clock `now_ms` (ADR 0139).
    #[must_use]
    pub fn from_headers(
        cache_control: Option<&str>,
        expires: Option<&str>,
        retry_after_header: Option<&str>,
        date: Option<&str>,
        switches: SiteHintSwitches,
        now_ms: u64,
    ) -> Self {
        let mut floor: Option<u64> = None;
        let mut raise = |candidate: Option<u64>| {
            if let Some(ts) = candidate {
                floor = Some(floor.map_or(ts, |current| current.max(ts)));
            }
        };
        if switches.cache_control
            && let Some(cc) = cache_control
        {
            raise(cache_control_max_age_floor(cc, now_ms));
        }
        if switches.expires
            && let Some(exp) = expires
        {
            raise(expires_floor(exp, date, now_ms));
        }
        if switches.retry_after
            && let Some(ra) = retry_after_header
        {
            raise(retry_after_floor(ra, date, now_ms));
        }
        Self { floor_at_ms: floor }
    }
}

fn cache_control_max_age_floor(header: &str, now_ms: u64) -> Option<u64> {
    for directive in header.split(',') {
        let Some((name, raw_val)) = directive.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("max-age") {
            let trimmed = raw_val.trim().trim_matches('"');
            let secs: u64 = trimmed.parse().ok()?;
            return (secs > 0).then(|| now_ms.saturating_add(secs.saturating_mul(1_000)));
        }
    }
    None
}

fn expires_floor(header: &str, date: Option<&str>, now_ms: u64) -> Option<u64> {
    let trimmed = header.trim_matches([' ', '\t']);
    // RFC 9111 §5.3: numeric values like `0` or `-1` mean already expired, not delta-seconds.
    if trimmed.is_empty() || trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    retry_after_floor(trimmed, date, now_ms)
}

fn retry_after_floor(header: &str, date: Option<&str>, now_ms: u64) -> Option<u64> {
    let received = UNIX_EPOCH.checked_add(Duration::from_millis(now_ms))?;
    match retry_after::read(header, date, received) {
        RetryAfter::Until(until) => {
            let ms = until.duration_since(UNIX_EPOCH).ok()?.as_millis();
            Some(u64::try_from(ms).unwrap_or(u64::MAX))
        }
        RetryAfter::NoWait | RetryAfter::Unparsable => None,
    }
}

/// Output of the pure schedule function: the chosen interval and next due time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheduled {
    /// Interval in milliseconds chosen by the policy (before any floor push).
    pub interval_ms: u64,
    /// Wall-clock timestamp (ms since Unix epoch) when the URL is next due.
    pub next_visit_at: u64,
    /// Whether the policy wanted an interval below `min` because the URL is
    /// changing faster than `min` allows (ADR 0139).
    pub clamped_at_min: bool,
}

/// Pure function computing when a URL is next due for a revisit (ADR 0139).
///
/// Returns `None` when the policy is `never`. Otherwise returns the interval
/// and next due timestamp in milliseconds since the Unix epoch, never earlier
/// than any site floor in `hints`.
#[must_use]
pub fn next_visit(
    policy: &RevisitPolicy,
    previous: PreviousState,
    result: VisitResult,
    hints: SiteHints,
    now_ms: u64,
) -> Option<Scheduled> {
    let (interval_ms, clamped_at_min) = match *policy {
        RevisitPolicy::Never => return None,
        RevisitPolicy::Every { interval } => (interval.max(1), false),
        RevisitPolicy::Adaptive {
            initial,
            min,
            max,
            rate_changed,
            rate_unchanged,
        } => adaptive_interval(
            initial,
            min,
            max,
            rate_changed,
            rate_unchanged,
            previous,
            result,
        ),
        RevisitPolicy::Estimated {
            initial,
            min,
            max,
            target_freshness,
        } => estimated_interval(initial, min, max, target_freshness, previous, result),
    };
    let policy_due = now_ms.saturating_add(interval_ms);
    let next_visit_at = hints
        .floor_at_ms
        .map_or(policy_due, |floor| policy_due.max(floor));
    Some(Scheduled {
        interval_ms,
        next_visit_at,
        clamped_at_min,
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "revisit intervals and visit counts fit well within f64 precision for schedule arithmetic"
)]
const fn to_f64(value: u64) -> f64 {
    value as f64
}

/// Multiplicative adaptive schedule (Nutch / `StormCrawler`, ADR 0139).
fn adaptive_interval(
    initial: u64,
    min: u64,
    max: u64,
    rate_changed: f64,
    rate_unchanged: f64,
    previous: PreviousState,
    result: VisitResult,
) -> (u64, bool) {
    let min = min.max(1);
    let max = max.max(min);
    let Some(prev_ms) = previous.interval_ms.filter(|_| previous.visits > 0) else {
        return (initial.clamp(min, max), false);
    };
    let raw = match result {
        VisitResult::Changed => to_f64(prev_ms) * (1.0 - rate_changed),
        VisitResult::Unchanged => to_f64(prev_ms) * (1.0 + rate_unchanged),
    };
    if !raw.is_finite() {
        return (max, false);
    }
    if result == VisitResult::Changed && raw < to_f64(min) {
        return (min, true);
    }
    if raw >= to_f64(max) {
        return (max, false);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "raw is finite and bounded within [0.0, max as f64]"
    )]
    let rounded = raw.round().max(0.0) as u64;
    (rounded.clamp(min, max), false)
}

/// Cho and Garcia-Molina's estimator `r̂ = -ln((n - X + 0.5)/(n + 0.5))`
/// with a prior of two intervals of `initial` (1 change in 2 intervals),
/// scheduled to reach expected freshness `(1 - e^{-λT}) / (λT) = target_freshness`
/// (ADR 0139).
fn estimated_interval(
    initial: u64,
    min: u64,
    max: u64,
    target_freshness: f64,
    previous: PreviousState,
    result: VisitResult,
) -> (u64, bool) {
    let min = min.max(1);
    let max = max.max(min);
    if previous.visits == 0 || previous.interval_ms.is_none() {
        return (initial.clamp(min, max), false);
    }
    let n_obs = previous.visits;
    let x_obs = previous
        .changes
        .saturating_sub(1)
        .saturating_add(u64::from(result == VisitResult::Changed))
        .min(n_obs);
    // Prior of two intervals of `initial` (neutral 1 change in 2 intervals).
    let n = to_f64(n_obs) + 2.0;
    let x = to_f64(x_obs) + 1.0;
    let r_hat = -((n - x + 0.5) / (n + 0.5)).ln();
    if !r_hat.is_finite() || r_hat <= 0.0 {
        return (max, false);
    }
    let prev_ms = to_f64(previous.interval_ms.unwrap_or(initial).max(1));
    let i_ref = (2.0 * to_f64(initial.max(1)) + to_f64(n_obs) * prev_ms) / n;
    let lambda_hat = r_hat / i_ref;
    let u = freshness_rate_product(target_freshness);
    let raw = u / lambda_hat;
    if !raw.is_finite() {
        return (max, false);
    }
    if raw < to_f64(min) {
        return (min, true);
    }
    if raw >= to_f64(max) {
        return (max, false);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "raw is finite and strictly between min and max"
    )]
    let rounded = raw.round() as u64;
    (rounded.clamp(min, max), false)
}

/// Solves `(1 - e^{-u}) / u = target_freshness` for `u > 0` by bisection.
fn freshness_rate_product(target_freshness: f64) -> f64 {
    let target = target_freshness.clamp(1e-9, 1.0 - 1e-9);
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    while (1.0 - (-hi).exp()) / hi > target {
        hi *= 2.0;
    }
    for _ in 0..64 {
        let mid = f64::midpoint(lo, hi);
        if mid == 0.0 {
            break;
        }
        if (1.0 - (-mid).exp()) / mid > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f64::midpoint(lo, hi)
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
             visits, changes, consecutive_gone, last_tier, at_min_floor
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
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
             last_tier = excluded.last_tier,
             at_min_floor = excluded.at_min_floor",
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
        i64::from(row.at_min_floor),
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
                visits, changes, consecutive_gone, last_tier, at_min_floor
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
                visits, changes, consecutive_gone, last_tier, at_min_floor
         FROM revisits ORDER BY work_item",
    )?;
    let rows = statement.query_map([], decode_row)?;
    rows.collect()
}

/// Returns the sorted identities of URLs currently changing faster than their
/// Revisit policy's `min` interval allows (ADR 0139).
///
/// # Errors
///
/// Returns SQLite errors from reading the table.
pub fn at_min_floor(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare_cached(
        "SELECT identity FROM revisits
         WHERE at_min_floor = 1
         ORDER BY identity ASC",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
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
        at_min_floor: row.get::<_, i64>(14)? != 0,
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
