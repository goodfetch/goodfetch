//! The Session pools in run state (ADRs 0002 and 0006): the tables of a
//! crawl's Sessions, each by its pool (the Label that owns it, or `''` for
//! the Run's) and the position it opened at there, with its proxy tier,
//! when it opened, its uses, its error score and, once retired, why; and of
//! each domain key a block moved, with its proxy tier and whether it moved
//! to the browser tier. The pools read them when their crawl starts or
//! resumes, and write what changed since their last write inside one of
//! the run-state writer's commits.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::session_pool::ProxyTier;
use rusqlite::Connection;

use crate::labels::PoolKey;

/// The tables of a crawl's Sessions and its domains' tiers.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-session-pool",
    statements: &[
        "CREATE TABLE IF NOT EXISTS pool_sessions (
            pool TEXT NOT NULL,
            position INTEGER NOT NULL,
            proxy_tier TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT,
            PRIMARY KEY (pool, position)
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS pool_sessions_open ON pool_sessions (pool, position)
            WHERE retired_because IS NULL",
        "CREATE TABLE IF NOT EXISTS pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            browser INTEGER NOT NULL
        ) STRICT",
    ],
};

/// One Session of a pool as run state keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredSession {
    pub(crate) proxy_tier: ProxyTier,
    pub(crate) opened_at: SystemTime,
    pub(crate) uses: u64,
    /// Its error score, in steps of a half, exact in binary.
    pub(crate) error_score: f64,
    /// Why it was retired; `None` while it is open.
    pub(crate) retired_because: Option<&'static str>,
}

/// Where a block moved one domain key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StoredTier {
    pub(crate) proxy_tier: ProxyTier,
    /// Whether a Challenge moved it to the browser tier.
    pub(crate) browser: bool,
}

impl Default for StoredTier {
    fn default() -> Self {
        Self {
            proxy_tier: ProxyTier::None,
            browser: false,
        }
    }
}

/// What the pools read from run state.
#[derive(Debug, Default)]
pub(crate) struct Stored {
    /// Their open Sessions, by pool and position.
    pub(crate) open: BTreeMap<(PoolKey, u64), StoredSession>,
    /// How many Sessions each pool ever opened.
    pub(crate) opened: BTreeMap<PoolKey, u64>,
    /// Each domain key a block moved.
    pub(crate) tiers: BTreeMap<String, StoredTier>,
}

/// Reads the pools of a run file whose tables exist.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Stored> {
    let opened = connection
        .prepare("SELECT pool, max(position) FROM pool_sessions GROUP BY pool")?
        .query_map([], |row| {
            Ok((pool(row.get::<_, String>(0)?), unsigned(row.get(1)?)))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let open = connection
        .prepare(
            "SELECT pool, position, proxy_tier, opened_at, uses, error_score FROM pool_sessions
             WHERE retired_because IS NULL ORDER BY pool, position",
        )?
        .query_map([], |row| {
            Ok((
                (pool(row.get::<_, String>(0)?), unsigned(row.get(1)?)),
                StoredSession {
                    proxy_tier: tier(&row.get::<_, String>(2)?)?,
                    opened_at: UNIX_EPOCH + Duration::from_millis(unsigned(row.get(3)?)),
                    uses: unsigned(row.get(4)?),
                    error_score: row.get(5)?,
                    retired_because: None,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let tiers = connection
        .prepare("SELECT domain_key, proxy_tier, browser FROM pool_tiers")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                StoredTier {
                    proxy_tier: tier(&row.get::<_, String>(1)?)?,
                    browser: row.get(2)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Stored {
        open,
        opened,
        tiers,
    })
}

/// What changed in the pools since their last write: the Sessions opened,
/// served, scored or retired, and the domain keys moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub(crate) sessions: BTreeMap<(PoolKey, u64), StoredSession>,
    pub(crate) tiers: BTreeMap<String, StoredTier>,
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
                (pool, position, proxy_tier, opened_at, uses, error_score, retired_because)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (pool, position) DO UPDATE SET
                uses = excluded.uses,
                error_score = excluded.error_score,
                retired_because = excluded.retired_because",
        )?;
        for ((pool, position), stored) in &self.sessions {
            let opened_at = stored
                .opened_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            session.execute((
                pool_name(pool),
                integer(*position),
                stored.proxy_tier.as_str(),
                i64::try_from(opened_at).unwrap_or(i64::MAX),
                integer(stored.uses),
                stored.error_score,
                stored.retired_because,
            ))?;
        }
        let mut tier = connection.prepare(
            "INSERT INTO pool_tiers (domain_key, proxy_tier, browser) VALUES (?1, ?2, ?3)
             ON CONFLICT (domain_key) DO UPDATE SET
                proxy_tier = excluded.proxy_tier,
                browser = excluded.browser",
        )?;
        for (domain_key, stored) in &self.tiers {
            tier.execute((domain_key, stored.proxy_tier.as_str(), stored.browser))?;
        }
        Ok(())
    }
}

/// How run state names `pool`: its Label, or `''` for the Run's.
fn pool_name(pool: &PoolKey) -> &str {
    match pool {
        PoolKey::Run => "",
        PoolKey::Label(label) => label,
    }
}

/// The pool run state names `name`.
fn pool(name: String) -> PoolKey {
    if name.is_empty() {
        PoolKey::Run
    } else {
        PoolKey::Label(name)
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
