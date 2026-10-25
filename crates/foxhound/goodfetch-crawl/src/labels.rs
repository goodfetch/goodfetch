//! The Labels a crawl declares (ADR 0093), kept in its run file as its
//! start declared them, so a resumed crawl takes each Label's settings
//! again: its `fetch.tier`, and the Session pool its `session.source`
//! gives it.

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_fetch_tier::LabelTiers;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_session_pool::LabelPools;
use rusqlite::{Connection, params};

/// What a crawl's declared Labels set: each one's tier, and which have a
/// Session pool of their own.
#[derive(Clone, Debug, Default)]
pub(crate) struct Declared {
    pub(crate) tiers: LabelTiers,
    pub(crate) pools: LabelPools,
}

impl Declared {
    /// What `labels` set.
    ///
    /// # Errors
    ///
    /// Those of [`LabelTiers::of`] and [`LabelPools::of`].
    pub(crate) fn of(labels: &BTreeMap<String, LabelDeclaration>) -> Result<Self, CallError> {
        Ok(Self {
            tiers: LabelTiers::of(labels)?,
            pools: LabelPools::of(labels)?,
        })
    }
}

/// The table that keeps a crawl's declared Labels in every run file.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl-labels",
    statements: &["CREATE TABLE IF NOT EXISTS labels (
            name TEXT NOT NULL PRIMARY KEY,
            declaration TEXT NOT NULL
        ) STRICT"],
};

/// Records `labels`, the Labels the crawl's start declares.
///
/// # Errors
///
/// The run file's, when the rows cannot be written.
pub(crate) fn record(
    connection: &Connection,
    labels: &BTreeMap<String, LabelDeclaration>,
) -> rusqlite::Result<()> {
    let mut insert =
        connection.prepare("INSERT OR REPLACE INTO labels (name, declaration) VALUES (?1, ?2)")?;
    for (name, declaration) in labels {
        let declaration = serde_json::to_string(declaration)
            .expect("invariant: a Label's declaration encodes as JSON");
        insert.execute(params![name, declaration])?;
    }
    Ok(())
}

/// The Labels the crawl's start declared, as [`record`] kept them; a row
/// that no longer reads is left out.
///
/// # Errors
///
/// The run file's, when the rows cannot be read.
pub(crate) fn read(
    connection: &Connection,
) -> rusqlite::Result<BTreeMap<String, LabelDeclaration>> {
    let mut select = connection.prepare("SELECT name, declaration FROM labels ORDER BY name")?;
    let rows = select.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut labels = BTreeMap::new();
    for row in rows {
        let (name, declaration) = row?;
        match serde_json::from_str(&declaration) {
            Ok(declaration) => {
                labels.insert(name, declaration);
            }
            Err(error) => {
                tracing::warn!(%error, label = %name, "a declared Label in the run file no longer reads");
            }
        }
    }
    Ok(labels)
}

#[cfg(test)]
mod tests;
