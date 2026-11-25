//! `dataset.erase` (spec #1937, story 214; ADR 0031): a person erases one
//! Dataset of a run no engine runs, at once and with no user code. Its
//! committed and rejected Records and their rows of the full-text index
//! are deleted in one write of the run's state, under its writer lock,
//! with `secure_delete` on, FTS5's included, so their pages are
//! overwritten; the engine then deletes the run's kept Exports. The crawl
//! still declares the Dataset, which reads empty from then on. The engine
//! answers it only to a person's surface, before this handler runs.

use goodfetch_engine::run::run_error;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::erase::{DATASET_ERASE, DatasetEraseParams, DatasetErased};
use rusqlite::Connection;

use crate::open::open_dataset;

/// The handler of `dataset.erase`.
pub(crate) fn handler() -> Handler {
    Handler::new(&DATASET_ERASE, erase)
}

async fn erase(
    params: DatasetEraseParams,
    context: CallContext,
) -> Result<DatasetErased, CallError> {
    drop(open_dataset(&context, params.run_id.clone(), &params.dataset).await?);
    let dataset = params.dataset.clone();
    let (records, rejected_records) = context
        .runs()
        .edit(params.run_id.clone(), move |connection| {
            erase_records(connection, &dataset)
        })
        .await
        .map_err(|error| run_error(error, &context))?;
    Ok(DatasetErased {
        run_id: params.run_id,
        dataset: params.dataset,
        records,
        rejected_records,
    })
}

/// Deletes the Records of `dataset`, committed and rejected, and its rows
/// of the full-text index, and answers how many of each it deleted.
/// The index's `secure-delete` option, which persists in the run file,
/// has its deleted rows' tokens removed rather than marked.
fn erase_records(connection: &Connection, dataset: &str) -> rusqlite::Result<(u64, u64)> {
    connection
        .prepare_cached(
            "INSERT INTO record_search(record_search, rank) VALUES ('secure-delete', 1)",
        )?
        .execute([])?;
    connection
        .prepare_cached("DELETE FROM record_search WHERE dataset = ?1")?
        .execute([dataset])?;
    let records = connection
        .prepare_cached("DELETE FROM records WHERE dataset = ?1")?
        .execute([dataset])?;
    let rejected = connection
        .prepare_cached("DELETE FROM rejected_records WHERE dataset = ?1")?
        .execute([dataset])?;
    Ok((count(records), count(rejected)))
}

/// A count of rows SQLite answers, which fits.
fn count(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(u64::MAX)
}
