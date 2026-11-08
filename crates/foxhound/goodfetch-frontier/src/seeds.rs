//! Each seed's answer (ADR 0102): the Frontier's answers to a crawl's seeds,
//! written in the transaction that records the Run, read back by
//! [`answers`], and `run.seeds`, which reads them from the Run's committed
//! state through the engine's Run reader, so they are the answers given
//! when the Run began, on a Run no program holds too.

use goodfetch_engine::run::run_error;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::frontier::{Enqueued, RUN_SEEDS, RunSeeds, RunSeedsParams};
use rusqlite::{Connection, OptionalExtension as _};

/// The table of a crawl's seeds' answers, by the seed's position from 1.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-seeds",
    statements: &["CREATE TABLE IF NOT EXISTS seeds (
            position INTEGER NOT NULL PRIMARY KEY,
            answer TEXT NOT NULL
        ) STRICT"],
};

/// Writes each seed's answer, in the order the start gave the seeds.
///
/// # Errors
///
/// When the run file cannot be written.
pub fn answered(connection: &Connection, answers: &[Enqueued]) -> rusqlite::Result<()> {
    let mut insert =
        connection.prepare_cached("INSERT INTO seeds (position, answer) VALUES (?1, ?2)")?;
    for (position, answer) in answers.iter().enumerate() {
        insert.execute((
            i64::try_from(position + 1).unwrap_or(i64::MAX),
            serde_json::to_string(answer).expect("invariant: an answer encodes as JSON"),
        ))?;
    }
    Ok(())
}

/// The handler of `run.seeds`.
pub(crate) fn handler() -> Handler {
    Handler::new(&RUN_SEEDS, seeds)
}

async fn seeds(params: RunSeedsParams, context: CallContext) -> Result<RunSeeds, CallError> {
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = context.runs().reader(params.run_id).await.map_err(failed)?;
    let seeds = reader.read(read).await.map_err(failed)?;
    Ok(RunSeeds { seeds })
}

/// Each seed's answer in order; none in a run file no crawl wrote.
fn read(connection: &Connection) -> rusqlite::Result<Vec<Enqueued>> {
    let has_seeds = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'seeds'")
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some();
    if !has_seeds {
        return Ok(Vec::new());
    }
    answers(connection)
}

/// Each seed's answer, in the order the start gave the seeds: the reader
/// other crates read the `seeds` table through.
///
/// # Errors
///
/// When the run file cannot be read, or an answer no longer decodes.
pub fn answers(connection: &Connection) -> rusqlite::Result<Vec<Enqueued>> {
    connection
        .prepare_cached("SELECT answer FROM seeds ORDER BY position")?
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|answer| {
            let answer = answer?;
            serde_json::from_str(&answer).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .collect()
}
