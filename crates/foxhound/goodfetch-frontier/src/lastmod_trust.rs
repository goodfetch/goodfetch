//! Per-host `<lastmod>` trust learning and claim judgement for Continuous
//! crawls (ADR 0139, spec #1836).
//!
//! A sitemap entry whose `<lastmod>` is later than a visited URL's
//! `last_fetched_at` opens one claim for that URL. Its next revisit judges the
//! claim: a content change confirms it; an unchanged revisit marks it false.
//! Once a host has at least `min_claims` judged claims and at least
//! `min_precision` of the last `window` claims are confirmed, the host earns
//! binary trust and its claims bring revisits forward to the latest of `now`,
//! site floors, and `last_fetched_at + policy.min`. Trust is withdrawn the
//! moment that share falls below `min_precision`, and can be earned again.
//!
//! A sitemap file of at least 10 entries with at least 90 % sharing one
//! `<lastmod>` date is treated as generated at build time (`lastmod_generated`):
//! it makes no claims and brings nothing forward.
//!
//! `news:publication_date` brings a revisit forward with no learning, and
//! counts its false claims per host.

use std::collections::BTreeMap;

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::continuous::{
    LASTMOD_TRUST_DEFAULT_MIN_CLAIMS, LASTMOD_TRUST_DEFAULT_MIN_PRECISION,
    LASTMOD_TRUST_DEFAULT_WINDOW, RevisitPolicy,
};
use goodfetch_protocol::foxhound::report::LastmodTrustHost;
use rusqlite::{Connection, OptionalExtension as _};

use crate::sitemap::{SitemapDate, SitemapEntry};

/// Whole UTC day in milliseconds.
const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

/// Minimum entries in one sitemap file before the stamped-at-generation check
/// applies (spec #1836).
const STAMPED_MIN_ENTRIES: usize = 10;

/// The `host_lastmod_trust` and `open_lastmod_claims` tables in every run file
/// (ADR 0139, spec #1836). Only a Continuous crawl writes rows into them.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-lastmod-trust",
    statements: &[
        "CREATE TABLE IF NOT EXISTS host_lastmod_trust (
            host TEXT NOT NULL PRIMARY KEY,
            domain_key TEXT NOT NULL,
            trusted INTEGER NOT NULL DEFAULT 0,
            claims_judged INTEGER NOT NULL DEFAULT 0,
            confirmed INTEGER NOT NULL DEFAULT 0,
            false_claims INTEGER NOT NULL DEFAULT 0,
            misses INTEGER NOT NULL DEFAULT 0,
            news_false_claims INTEGER NOT NULL DEFAULT 0,
            window_outcomes TEXT NOT NULL DEFAULT ''
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS open_lastmod_claims (
            work_item TEXT NOT NULL PRIMARY KEY,
            host TEXT NOT NULL,
            domain_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            claimed_at_ms INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID",
    ],
};

/// Whether an open claim came from `<lastmod>` (learned) or an unlearned
/// freshness hint such as `<news:publication_date>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimKind {
    /// Sitemap `<lastmod>`, subject to per-host trust learning.
    Lastmod,
    /// `<news:publication_date>` (or feed update date): brings revisits forward
    /// with no learning and counts false claims per host.
    News,
}

impl ClaimKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lastmod => "lastmod",
            Self::News => "news",
        }
    }

    fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "lastmod" => Some(Self::Lastmod),
            "news" => Some(Self::News),
            _ => None,
        }
    }
}

/// Thresholds controlling when a host earns and keeps `<lastmod>` trust
/// (`crawl.sitemap.lastmod_trust.*`, spec #1836).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrustThresholds {
    /// Minimum judged claims before a host can be trusted (default: 10).
    pub min_claims: u64,
    /// Minimum share of confirmed claims in the rolling window (default: 0.9).
    pub min_precision: f64,
    /// Rolling window length of recent judged claims (default: 20).
    pub window: usize,
}

impl Default for TrustThresholds {
    fn default() -> Self {
        Self {
            min_claims: LASTMOD_TRUST_DEFAULT_MIN_CLAIMS,
            min_precision: LASTMOD_TRUST_DEFAULT_MIN_PRECISION,
            window: usize::try_from(LASTMOD_TRUST_DEFAULT_WINDOW).unwrap_or(20),
        }
    }
}

/// Returns `true` when `entries` has at least 10 entries and at least 90 %
/// share the exact same `<lastmod>` date (stamped at sitemap generation time,
/// spec #1836).
#[must_use]
pub fn is_stamped_sitemap(entries: &[SitemapEntry]) -> bool {
    let total = entries.len();
    if total < STAMPED_MIN_ENTRIES {
        return false;
    }
    let mut counts: BTreeMap<(u64, bool), usize> = BTreeMap::new();
    let mut max_same = 0usize;
    for entry in entries {
        if let Some(date) = entry.lastmod {
            let count = counts.entry((date.ms, date.date_only)).or_insert(0);
            *count += 1;
            max_same = max_same.max(*count);
        }
    }
    max_same.saturating_mul(10) >= total.saturating_mul(9)
}

/// Returns `true` when `date` is a valid claim later than `last_fetched_at_ms`
/// at wall-clock `now_ms` (spec #1836):
/// - A date more than 24 hours ahead of `now_ms` is ignored.
/// - A date-only `<lastmod>` compares by whole UTC day.
/// - Otherwise compares by exact millisecond timestamp.
#[must_use]
pub const fn is_claim_later(date: SitemapDate, last_fetched_at_ms: u64, now_ms: u64) -> bool {
    if date.ms > now_ms.saturating_add(DAY_MS) {
        return false;
    }
    if date.date_only {
        date.ms / DAY_MS > last_fetched_at_ms / DAY_MS
    } else {
        date.ms > last_fetched_at_ms
    }
}

/// Computes the brought-forward due timestamp for a trusted claim (spec #1836):
/// the latest of `now_ms`, any site `floor_at_ms`, and
/// `last_fetched_at_ms + policy.min`. Returns `None` for `RevisitPolicy::Never`
/// (a `never` URL stays never).
#[must_use]
pub fn brought_forward_due(
    now_ms: u64,
    last_fetched_at_ms: u64,
    floor_at_ms: Option<u64>,
    policy: &RevisitPolicy,
) -> Option<u64> {
    let min_ms = match *policy {
        RevisitPolicy::Never => return None,
        RevisitPolicy::Every { interval } => interval,
        RevisitPolicy::Adaptive { min, .. } | RevisitPolicy::Estimated { min, .. } => min,
    };
    let mut due = now_ms.max(last_fetched_at_ms.saturating_add(min_ms));
    if let Some(floor) = floor_at_ms {
        due = due.max(floor);
    }
    Some(due)
}

/// Opens (or replaces) the single open claim for `work_item` (one claim is open
/// per URL, spec #1836).
///
/// # Errors
///
/// Returns SQLite errors from writing `open_lastmod_claims`.
pub fn open_claim(
    connection: &Connection,
    work_item: &str,
    host: &str,
    domain_key: &str,
    kind: ClaimKind,
    claimed_at_ms: u64,
) -> rusqlite::Result<()> {
    let ts = i64::try_from(claimed_at_ms).unwrap_or(i64::MAX);
    connection
        .prepare_cached(
            "INSERT INTO open_lastmod_claims (work_item, host, domain_key, kind, claimed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (work_item) DO UPDATE SET
               host = excluded.host,
               domain_key = excluded.domain_key,
               kind = excluded.kind,
               claimed_at_ms = excluded.claimed_at_ms",
        )?
        .execute((work_item, host, domain_key, kind.as_str(), ts))?;
    Ok(())
}

/// Returns whether `host` is currently trusted in `host_lastmod_trust`.
///
/// # Errors
///
/// Returns SQLite errors from reading `host_lastmod_trust`.
pub fn is_host_trusted(connection: &Connection, host: &str) -> rusqlite::Result<bool> {
    let trusted: Option<i64> = connection
        .prepare_cached("SELECT trusted FROM host_lastmod_trust WHERE host = ?1")?
        .query_row([host], |row| row.get(0))
        .optional()?;
    Ok(trusted.is_some_and(|v| v != 0))
}

/// Evaluates whether a host should be trusted given `claims_judged`, its
/// rolling `window_outcomes` (`'1'` confirmed, `'0'` false), and `thresholds`.
#[must_use]
pub fn evaluate_trust(
    claims_judged: u64,
    window_outcomes: &str,
    thresholds: TrustThresholds,
) -> bool {
    if claims_judged < thresholds.min_claims || window_outcomes.is_empty() {
        return false;
    }
    let confirmed = window_outcomes.bytes().filter(|&b| b == b'1').count();
    let total = window_outcomes.len();
    #[expect(
        clippy::cast_precision_loss,
        reason = "rolling window length is tiny (default 20), exact in f64"
    )]
    let precision = (confirmed as f64) / (total as f64);
    precision >= thresholds.min_precision
}

/// Judges any open claim for `work_item` when a revisit completes, or records a
/// miss when a revisit saw a content change with no open claim (ADR 0139,
/// spec #1836).
///
/// Runs inside the Run-state batch transaction that commits the outcome (0
/// extra transactions, ADR 0140 F2 / ADR 0142).
///
/// # Errors
///
/// Returns SQLite errors from updating `open_lastmod_claims` and
/// `host_lastmod_trust`.
pub fn judge_visit(
    connection: &Connection,
    work_item: &str,
    host: &str,
    domain_key: &str,
    changed: bool,
    is_revisit: bool,
    thresholds: TrustThresholds,
) -> rusqlite::Result<()> {
    let open: Option<(String, String, String)> = connection
        .prepare_cached(
            "SELECT host, domain_key, kind FROM open_lastmod_claims WHERE work_item = ?1",
        )?
        .query_row([work_item], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()?;

    if let Some((claim_host, claim_domain, kind_raw)) = open {
        connection
            .prepare_cached("DELETE FROM open_lastmod_claims WHERE work_item = ?1")?
            .execute([work_item])?;
        match ClaimKind::from_str(&kind_raw) {
            Some(ClaimKind::Lastmod) => {
                record_lastmod_judgement(
                    connection,
                    &claim_host,
                    &claim_domain,
                    changed,
                    thresholds,
                )?;
            }
            Some(ClaimKind::News) if !changed => {
                record_news_false_claim(connection, &claim_host, &claim_domain)?;
            }
            Some(ClaimKind::News) | None => {}
        }
        return Ok(());
    }

    if is_revisit && changed && !host.is_empty() {
        record_miss(connection, host, domain_key)?;
    }
    Ok(())
}

fn record_lastmod_judgement(
    connection: &Connection,
    host: &str,
    domain_key: &str,
    changed: bool,
    thresholds: TrustThresholds,
) -> rusqlite::Result<()> {
    let existing: Option<(i64, i64, i64, String)> = connection
        .prepare_cached(
            "SELECT claims_judged, confirmed, false_claims, window_outcomes
             FROM host_lastmod_trust WHERE host = ?1",
        )?
        .query_row([host], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .optional()?;

    let (mut judged, mut confirmed, mut false_claims, mut window_outcomes) =
        existing.map_or((0_u64, 0_u64, 0_u64, String::new()), |(j, c, f, w)| {
            (
                u64::try_from(j).unwrap_or(0),
                u64::try_from(c).unwrap_or(0),
                u64::try_from(f).unwrap_or(0),
                w,
            )
        });

    judged = judged.saturating_add(1);
    if changed {
        confirmed = confirmed.saturating_add(1);
        window_outcomes.push('1');
    } else {
        false_claims = false_claims.saturating_add(1);
        window_outcomes.push('0');
    }
    let cap = thresholds.window.max(1);
    if window_outcomes.len() > cap {
        let drop_n = window_outcomes.len() - cap;
        window_outcomes.drain(..drop_n);
    }
    let trusted = evaluate_trust(judged, &window_outcomes, thresholds);

    connection
        .prepare_cached(
            "INSERT INTO host_lastmod_trust
                 (host, domain_key, trusted, claims_judged, confirmed, false_claims, window_outcomes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (host) DO UPDATE SET
                 domain_key = excluded.domain_key,
                 trusted = excluded.trusted,
                 claims_judged = excluded.claims_judged,
                 confirmed = excluded.confirmed,
                 false_claims = excluded.false_claims,
                 window_outcomes = excluded.window_outcomes",
        )?
        .execute((
            host,
            domain_key,
            i64::from(trusted),
            i64::try_from(judged).unwrap_or(i64::MAX),
            i64::try_from(confirmed).unwrap_or(i64::MAX),
            i64::try_from(false_claims).unwrap_or(i64::MAX),
            &window_outcomes,
        ))?;
    Ok(())
}

fn record_miss(connection: &Connection, host: &str, domain_key: &str) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO host_lastmod_trust (host, domain_key, misses)
             VALUES (?1, ?2, 1)
             ON CONFLICT (host) DO UPDATE SET misses = host_lastmod_trust.misses + 1",
        )?
        .execute((host, domain_key))?;
    Ok(())
}

fn record_news_false_claim(
    connection: &Connection,
    host: &str,
    domain_key: &str,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO host_lastmod_trust (host, domain_key, news_false_claims)
             VALUES (?1, ?2, 1)
             ON CONFLICT (host) DO UPDATE SET
               news_false_claims = host_lastmod_trust.news_false_claims + 1",
        )?
        .execute((host, domain_key))?;
    Ok(())
}

/// Snapshot of host `<lastmod>` trust rows for the Freshness report (ADR 0139).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustSummary {
    /// Sorted hosts whose `<lastmod>` is currently trusted.
    pub trusted_hosts: Vec<String>,
    /// Sorted trusted hosts grouped by `domain_key`.
    pub trusted_by_domain: BTreeMap<String, Vec<String>>,
    /// Per-host trust ledger rows (for hosts with at least one judged claim or miss).
    pub hosts: Vec<LastmodTrustHost>,
    /// Per-host false claims from unlearned `news:publication_date` / feed hints.
    pub news_false_claims: BTreeMap<String, u64>,
}

/// Reads the host trust ledger from `connection` for `FreshnessReport`
/// (ADR 0139, spec #1836).
///
/// # Errors
///
/// Returns SQLite errors from reading `host_lastmod_trust`.
pub fn summary(connection: &Connection) -> rusqlite::Result<TrustSummary> {
    let has_table = connection
        .prepare_cached(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'host_lastmod_trust'",
        )?
        .query_row([], |_| Ok(()))
        .optional()?
        .is_some();
    if !has_table {
        return Ok(TrustSummary::default());
    }
    let mut statement = connection.prepare_cached(
        "SELECT host, domain_key, trusted, claims_judged, confirmed, false_claims, misses, news_false_claims
         FROM host_lastmod_trust ORDER BY host",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)? != 0,
            u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
            u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
            u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
            u64::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
            u64::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
        ))
    })?;
    let mut out = TrustSummary::default();
    for row in rows {
        let (host, domain_key, trusted, claims_judged, confirmed, false_claims, misses, news_false) =
            row?;
        if trusted {
            out.trusted_hosts.push(host.clone());
            out.trusted_by_domain
                .entry(domain_key)
                .or_default()
                .push(host.clone());
        }
        if claims_judged > 0 || misses > 0 {
            out.hosts.push(LastmodTrustHost {
                host: host.clone(),
                trusted,
                claims_judged,
                confirmed,
                false_claims,
                misses,
            });
        }
        if news_false > 0 {
            out.news_false_claims.insert(host, news_false);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
