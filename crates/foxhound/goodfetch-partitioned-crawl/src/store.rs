//! Persistence for Partitioned crawls:
//! - Lead Daemon user store (`partitioned-crawls.sqlite`) recording each
//!   Partitioned crawl's ID, Partition map, and Partition Run IDs so any
//!   `run.report`, `run.stop`, `run.abort`, `run.pause`, `run.checkpoint`,
//!   `run.resume`, or `run.records` call survives Daemon restarts (ADR 0054).
//! - Per-Partition run-state table (`partitioned_crawl_membership`) so a
//!   resumed Partition recovers its `LimitScope { kind: "partitioned_crawl", id }`
//!   automatically through `Contribution::with_limit_scope_reader` (ADR 0041,
//!   0054 and 0087).

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::user_store::{UserStore, UserStoreError, UserStoreKind};
use goodfetch_protocol::catalog::{ProtocolVersion, schema_of};
use goodfetch_protocol::formats::{Format, Rule, Storage};
use goodfetch_protocol::foxhound::usage::PARTITIONED_CRAWL;
use goodfetch_protocol::run::{LimitScope, RunId};
use rusqlite::{Connection, OptionalExtension as _};
use serde::{Deserialize, Serialize};

use crate::rendezvous::PartitionMap;

/// The Partitioned crawl store's format (ADR 0054 and ADR 0098).
const PARTITIONED_CRAWL_STORE: Format = Format {
    name: "partitioned-crawl-store",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::SharedStore,
    hello_from: &[],
    schema: schema_of::<serde_json::Value>,
};

const STORE_TABLES: &[&str] = &["partitioned_crawls"];

/// SQLite DDL statements creating the Partitioned crawl user store tables.
pub const STORE_STATEMENTS: &[&str] = &["CREATE TABLE IF NOT EXISTS partitioned_crawls (
        crawl_id TEXT NOT NULL PRIMARY KEY,
        map_json TEXT NOT NULL,
        created_at INTEGER NOT NULL
    ) STRICT"];

/// The per-major user store holding Partitioned crawl coordination state on the
/// Lead Daemon.
pub static STORE: UserStoreKind = UserStoreKind::new(
    "partitioned-crawls.sqlite",
    &PARTITIONED_CRAWL_STORE,
    STORE_TABLES,
    STORE_STATEMENTS,
    "partitioned crawl store",
);

/// The run-state tables added to every Foxhound run file to remember which
/// Partitioned crawl a Partition Run belongs to (so resume re-attaches the
/// `partitioned_crawl` Limit scope and partition map) and any per-domain state
/// other areas attach to a domain key (ADR 0054).
pub const RUN_STATE_TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-partitioned-crawl",
    statements: &[
        "CREATE TABLE IF NOT EXISTS partitioned_crawl_membership (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            crawl_id TEXT NOT NULL,
            partition_index INTEGER NOT NULL,
            map_json TEXT NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS partition_domain_state (
            domain_key TEXT NOT NULL,
            area TEXT NOT NULL,
            state_json TEXT NOT NULL,
            PRIMARY KEY (domain_key, area)
        ) STRICT",
    ],
};

/// Durable record of one Partitioned crawl on the Lead Daemon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionedCrawlRecord {
    /// The Partitioned crawl ID.
    pub crawl_id: RunId,
    /// Current Partition map (including `map_epoch` and each Partition's Run ID
    /// and Group member).
    pub map: PartitionMap,
    /// Unix epoch milliseconds when created.
    pub created_at: i64,
}

/// Records `membership` inside a Partition's own run-state SQLite file.
///
/// # Errors
///
/// SQLite errors while writing the membership row.
pub fn write_partition_membership(
    connection: &Connection,
    crawl_id: &RunId,
    partition_index: u32,
    map: &PartitionMap,
) -> rusqlite::Result<()> {
    let map_json = serde_json::to_string(map).expect("invariant: PartitionMap encodes as JSON");
    connection
        .prepare_cached(
            "INSERT INTO partitioned_crawl_membership (id, crawl_id, partition_index, map_json)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 crawl_id = excluded.crawl_id,
                 partition_index = excluded.partition_index,
                 map_json = excluded.map_json",
        )?
        .execute((crawl_id.as_str(), i64::from(partition_index), map_json))?;
    Ok(())
}

/// Reads the Partition membership row from a Partition's run file, if present.
///
/// # Errors
///
/// SQLite errors while querying.
pub fn read_partition_membership(
    connection: &Connection,
) -> rusqlite::Result<Option<(RunId, u32, PartitionMap)>> {
    let row: Option<(String, i64, String)> = connection
        .prepare_cached(
            "SELECT crawl_id, partition_index, map_json FROM partitioned_crawl_membership WHERE id = 1",
        )?
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()?;
    let Some((crawl_id_str, idx, map_json)) = row else {
        return Ok(None);
    };
    let Ok(crawl_id) = RunId::parse(&crawl_id_str) else {
        return Ok(None);
    };
    let Ok(map) = serde_json::from_str::<PartitionMap>(&map_json) else {
        return Ok(None);
    };
    Ok(Some((
        crawl_id,
        u32::try_from(idx).unwrap_or_default(),
        map,
    )))
}

/// `LimitScopeReader` callback for `Contribution::with_limit_scope_reader`:
/// returns `LimitScope { kind: "partitioned_crawl", id }` when the opened Run
/// belongs to a Partitioned crawl.
///
/// # Errors
///
/// SQLite errors while querying.
pub fn read_partition_limit_scopes(connection: &Connection) -> rusqlite::Result<Vec<LimitScope>> {
    let crawl_id: Option<String> = connection
        .prepare_cached("SELECT crawl_id FROM partitioned_crawl_membership WHERE id = 1")?
        .query_row([], |row| row.get(0))
        .optional()?;
    Ok(crawl_id
        .map(|id| LimitScope {
            kind: PARTITIONED_CRAWL.to_owned(),
            id,
        })
        .into_iter()
        .collect())
}

/// Persists `record` into the Lead Daemon's `partitioned-crawls` user store.
///
/// # Errors
///
/// Store read/write errors.
pub async fn save_record(
    store: &UserStore,
    record: PartitionedCrawlRecord,
) -> Result<(), UserStoreError> {
    let crawl_id = record.crawl_id.as_str().to_owned();
    let map_json =
        serde_json::to_string(&record.map).expect("invariant: PartitionMap encodes as JSON");
    let created_at = record.created_at;
    store
        .write(move |connection| {
            connection
                .prepare_cached(
                    "INSERT INTO partitioned_crawls (crawl_id, map_json, created_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT (crawl_id) DO UPDATE SET map_json = excluded.map_json",
                )?
                .execute((crawl_id, map_json, created_at))?;
            Ok(())
        })
        .await
}

/// Loads a Partitioned crawl record by `crawl_id` from the Lead Daemon's store.
///
/// # Errors
///
/// Store read errors.
pub async fn load_record(
    store: &UserStore,
    crawl_id: &RunId,
) -> Result<Option<PartitionedCrawlRecord>, UserStoreError> {
    let key = crawl_id.as_str().to_owned();
    let expected = crawl_id.clone();
    store
        .read(None, move |connection| {
            let row: Option<(String, i64)> = connection
                .prepare_cached(
                    "SELECT map_json, created_at FROM partitioned_crawls WHERE crawl_id = ?1",
                )?
                .query_row([key], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            let Some((map_json, created_at)) = row else {
                return Ok(None);
            };
            let map: PartitionMap = serde_json::from_str(&map_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(Some(PartitionedCrawlRecord {
                crawl_id: expected,
                map,
                created_at,
            }))
        })
        .await
}
