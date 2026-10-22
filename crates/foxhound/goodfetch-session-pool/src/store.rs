//! The Session pool in run state (ADRs 0002 and 0006): the tables of a
//! crawl's Sessions, each by the position it opened at, with its proxy
//! tier, when it opened, its uses, its error score and, once retired, why;
//! and of each domain key's proxy tier once a block moved it up. The pool
//! reads them when its crawl starts or resumes, and writes what changed
//! since its last write inside one of the run-state writer's commits.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::session_pool::ProxyTier;
use rusqlite::Connection;

/// The tables of a crawl's Sessions and its domains' proxy tiers.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-session-pool",
    statements: &[
        "CREATE TABLE IF NOT EXISTS pool_sessions (
            position INTEGER NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS pool_sessions_open ON pool_sessions (position)
            WHERE retired_because IS NULL",
        "CREATE TABLE IF NOT EXISTS pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL
        ) STRICT",
    ],
};

/// One Session of the pool as run state keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredSession {
    /// The position it opened at, from 1, which names it.
    pub(crate) position: u64,
    pub(crate) proxy_tier: ProxyTier,
    pub(crate) opened_at: SystemTime,
    pub(crate) uses: u64,
    /// Its error score, in steps of a half, exact in binary.
    pub(crate) error_score: f64,
    /// Why it was retired; `None` while it is open.
    pub(crate) retired_because: Option<&'static str>,
}

/// What a pool reads from run state.
#[derive(Debug, Default)]
pub(crate) struct Stored {
    /// Its open Sessions, by position.
    pub(crate) open: BTreeMap<u64, StoredSession>,
    /// How many Sessions it ever opened.
    pub(crate) opened: u64,
    /// Each domain key a block moved up, with its proxy tier.
    pub(crate) tiers: BTreeMap<String, ProxyTier>,
}

/// Reads the pool of a run file whose tables exist.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Stored> {
    let opened: i64 = connection.query_row(
        "SELECT coalesce(max(position), 0) FROM pool_sessions",
        [],
        |row| row.get(0),
    )?;
    let open = connection
        .prepare(
            "SELECT position, proxy_tier, opened_at, uses, error_score FROM pool_sessions
             WHERE retired_because IS NULL ORDER BY position",
        )?
        .query_map([], |row| {
            Ok(StoredSession {
                position: unsigned(row.get(0)?),
                proxy_tier: tier(&row.get::<_, String>(1)?)?,
                opened_at: UNIX_EPOCH + Duration::from_millis(unsigned(row.get(2)?)),
                uses: unsigned(row.get(3)?),
                error_score: row.get(4)?,
                retired_because: None,
            })
        })?
        .map(|session| session.map(|session| (session.position, session)))
        .collect::<rusqlite::Result<_>>()?;
    let tiers = connection
        .prepare("SELECT domain_key, proxy_tier FROM pool_tiers")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, tier(&row.get::<_, String>(1)?)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Stored {
        open,
        opened: unsigned(opened),
        tiers,
    })
}

/// What changed in a pool since its last write: the Sessions opened,
/// served, scored or retired, and the domain keys moved up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub(crate) sessions: BTreeMap<u64, StoredSession>,
    pub(crate) tiers: BTreeMap<String, ProxyTier>,
}

impl Changes {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.tiers.is_empty()
    }

    /// Writes the changes into the run file, inside the caller's commit.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub fn write(&self, connection: &Connection) -> rusqlite::Result<()> {
        let mut session = connection.prepare(
            "INSERT INTO pool_sessions
                (position, proxy_tier, opened_at, uses, error_score, retired_because)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (position) DO UPDATE SET
                uses = excluded.uses,
                error_score = excluded.error_score,
                retired_because = excluded.retired_because",
        )?;
        for stored in self.sessions.values() {
            let opened_at = stored
                .opened_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            session.execute((
                integer(stored.position),
                stored.proxy_tier.as_str(),
                i64::try_from(opened_at).unwrap_or(i64::MAX),
                integer(stored.uses),
                stored.error_score,
                stored.retired_because,
            ))?;
        }
        let mut tier = connection.prepare(
            "INSERT INTO pool_tiers (domain_key, proxy_tier) VALUES (?1, ?2)
             ON CONFLICT (domain_key) DO UPDATE SET proxy_tier = excluded.proxy_tier",
        )?;
        for (domain_key, proxy_tier) in &self.tiers {
            tier.execute((domain_key, proxy_tier.as_str()))?;
        }
        Ok(())
    }
}

fn tier(name: &str) -> rusqlite::Result<ProxyTier> {
    ProxyTier::parse(name).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("`{name}` is no proxy tier").into(),
        )
    })
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
