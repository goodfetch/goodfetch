//! The Session pools in run state (ADRs 0002 and 0006): the tables of a
//! crawl's Sessions, each by its pool (the Label that owns it, or `''` for
//! the Run's) and the position it opened at there, with its proxy tier,
//! the client it sends from, when it opened, its uses, its error score and, once retired, why; and of
//! each domain key a block moved, with its proxy tier and whether it moved
//! to the browser tier; and of the local-address blocks a block retired a
//! Session of on each domain key (ADR 0044). The pools read them when their crawl starts or
//! resumes, and write what changed since their last write inside one of
//! the run-state writer's commits.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::session_pool::ProxyTier;
use rusqlite::Connection;

use crate::labels::PoolKey;

/// The tables of a crawl's Sessions and its domains' tiers.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-session-pool",
    statements: &[
        "CREATE TABLE IF NOT EXISTS pool_sessions (
            pool TEXT NOT NULL,
            position INTEGER NOT NULL,
            proxy_tier TEXT NOT NULL,
            sends_from TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT,
            address_ordinal INTEGER,
            PRIMARY KEY (pool, position)
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS pool_sessions_open ON pool_sessions (pool, position)
            WHERE retired_because IS NULL",
        "CREATE TABLE IF NOT EXISTS pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            browser INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS pool_blocked_address_blocks (
            domain_key TEXT NOT NULL,
            block TEXT NOT NULL,
            PRIMARY KEY (domain_key, block)
        ) STRICT",
    ],
};

/// Why a run file whose pool table predates the Sessions' client is not
/// resumed, and what to do instead.
pub(crate) const PREDATES_CLIENTS: &str = "its Session pool was stored by an earlier build of Goodfetch, before each Session kept whether it sends from its own HTTP transport or from its browser, so the crawl cannot resume: start a new Run of it";

/// Where a pool Session sends its requests from, for its life: its own
/// HTTP transport, or its browser. It takes one at its first use, run state
/// keeps it, and a request is only ever served by a Session of the client
/// it needs, so a site never meets one Session from both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SendsFrom {
    /// Its own HTTP transport.
    Transport,
    /// Its browser.
    Browser,
}

impl SendsFrom {
    /// Its name in run state.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Browser => "browser",
        }
    }

    /// The client run state names `name`, if it names one.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "transport" => Some(Self::Transport),
            "browser" => Some(Self::Browser),
            _ => None,
        }
    }
}

/// One Session of a pool as run state keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredSession {
    pub(crate) proxy_tier: ProxyTier,
    pub(crate) sends_from: SendsFrom,
    pub(crate) opened_at: SystemTime,
    pub(crate) uses: u64,
    /// Its error score, in steps of a half, exact in binary.
    pub(crate) error_score: f64,
    /// Why it was retired; `None` while it is open.
    pub(crate) retired_because: Option<&'static str>,
    /// The position of the Run's local-address rotation it took its
    /// addresses from; `None` without `egress.local_addresses`.
    pub(crate) address_ordinal: Option<u64>,
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
    /// The local-address rotation's next position.
    pub(crate) next_ordinal: u64,
    /// The address blocks blocked on each domain key.
    pub(crate) blocked: BTreeMap<String, BTreeSet<String>>,
}

/// Reads the pools of a run file whose tables exist.
///
/// # Errors
///
/// The run file's, and [`PREDATES_CLIENTS`] when its pool table was made
/// before Sessions kept their client: no reader of that shape exists.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Stored> {
    let keeps_clients = connection
        .prepare_cached(
            "SELECT count(*) FROM pragma_table_info('pool_sessions') WHERE name = 'sends_from'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))?;
    if keeps_clients == 0 {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(PREDATES_CLIENTS.to_owned()),
        ));
    }
    let opened = connection
        .prepare_cached("SELECT pool, max(position) FROM pool_sessions GROUP BY pool")?
        .query_map([], |row| {
            Ok((pool(row.get::<_, String>(0)?), unsigned(row.get(1)?)))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let open = connection
        .prepare_cached(
            "SELECT pool, position, proxy_tier, sends_from, opened_at, uses, error_score,
                address_ordinal
             FROM pool_sessions
             WHERE retired_because IS NULL ORDER BY pool, position",
        )?
        .query_map([], |row| {
            Ok((
                (pool(row.get::<_, String>(0)?), unsigned(row.get(1)?)),
                StoredSession {
                    proxy_tier: tier(&row.get::<_, String>(2)?)?,
                    sends_from: sends_from(&row.get::<_, String>(3)?)?,
                    opened_at: UNIX_EPOCH + Duration::from_millis(unsigned(row.get(4)?)),
                    uses: unsigned(row.get(5)?),
                    error_score: row.get(6)?,
                    retired_because: None,
                    address_ordinal: row.get::<_, Option<i64>>(7)?.map(unsigned),
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let tiers = connection
        .prepare_cached("SELECT domain_key, proxy_tier, browser FROM pool_tiers")?
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
    let next_ordinal = connection
        .prepare_cached("SELECT max(address_ordinal) + 1 FROM pool_sessions")
        .and_then(|mut cached| cached.query_row([], |row| row.get::<_, Option<i64>>(0)))?;
    let mut blocked: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut blocks =
        connection.prepare_cached("SELECT domain_key, block FROM pool_blocked_address_blocks")?;
    for row in blocks.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (domain_key, block) = row?;
        blocked.entry(domain_key).or_default().insert(block);
    }
    Ok(Stored {
        open,
        opened,
        tiers,
        next_ordinal: next_ordinal.map_or(0, unsigned),
        blocked,
    })
}

/// What changed in the pools since their last write: the Sessions opened,
/// served, scored or retired, and the domain keys moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub(crate) sessions: BTreeMap<(PoolKey, u64), StoredSession>,
    pub(crate) tiers: BTreeMap<String, StoredTier>,
    /// The address blocks newly blocked, by domain key.
    pub(crate) blocked: BTreeSet<(String, String)>,
}

impl Changes {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.tiers.is_empty() && self.blocked.is_empty()
    }

    /// Writes the changes into the run file, inside the caller's commit.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub fn write(&self, connection: &Connection) -> rusqlite::Result<()> {
        let mut session = connection.prepare_cached(
            "INSERT INTO pool_sessions
                (pool, position, proxy_tier, sends_from, opened_at, uses, error_score,
                 retired_because, address_ordinal)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
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
                stored.sends_from.as_str(),
                i64::try_from(opened_at).unwrap_or(i64::MAX),
                integer(stored.uses),
                stored.error_score,
                stored.retired_because,
                stored.address_ordinal.map(integer),
            ))?;
        }
        let mut tier = connection.prepare_cached(
            "INSERT INTO pool_tiers (domain_key, proxy_tier, browser) VALUES (?1, ?2, ?3)
             ON CONFLICT (domain_key) DO UPDATE SET
                proxy_tier = excluded.proxy_tier,
                browser = excluded.browser",
        )?;
        for (domain_key, stored) in &self.tiers {
            tier.execute((domain_key, stored.proxy_tier.as_str(), stored.browser))?;
        }
        let mut block = connection.prepare_cached(
            "INSERT OR IGNORE INTO pool_blocked_address_blocks (domain_key, block) VALUES (?1, ?2)",
        )?;
        for (domain_key, blocked) in &self.blocked {
            block.execute((domain_key, blocked))?;
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

fn sends_from(name: &str) -> rusqlite::Result<SendsFrom> {
    SendsFrom::parse(name).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("`{name}` is no client a Session sends from").into(),
        )
    })
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
