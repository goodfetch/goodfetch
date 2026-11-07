//! How every read of a Run's Datasets opens (ADRs 0008 and 0103): through
//! the engine's Run reader, on a read-only connection of its own, never the
//! Run's writer, once the run file proves the Run exists and, for a read of
//! one Dataset, that the crawl declares it, else `dataset_unknown`, a Run
//! no crawl wrote included, which declares none. Whether a call is first
//! forwarded to the Run's owner (ADR 0102) is each operation's own.

use goodfetch_engine::run::run_error;
use goodfetch_engine::runs::RunReader;
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::run::RunId;

use crate::declaration::Declarations;

/// The reader of the Run `run_id`, once its run file proves it exists.
///
/// # Errors
///
/// The Run's Error code (`run_not_found`, `run_state_unavailable`, …) when
/// it cannot be read.
///
/// # Cancel safety
///
/// Cancel safe: it only reads.
pub async fn open_run(context: &CallContext, run_id: RunId) -> Result<RunReader, CallError> {
    let mut reader = context
        .runs()
        .reader(run_id)
        .await
        .map_err(|error| run_error(error, context.identity()))?;
    reader
        .read(|_| Ok(()))
        .await
        .map_err(|error| run_error(error, context.identity()))?;
    Ok(reader)
}

/// The reader of the Run `run_id` for a read of its Dataset `dataset`, once
/// its run file proves the crawl declares it.
///
/// # Errors
///
/// As [`open_run`], and `dataset_unknown` when the crawl does not declare
/// `dataset`.
///
/// # Cancel safety
///
/// Cancel safe: it only reads.
pub(crate) async fn open_dataset(
    context: &CallContext,
    run_id: RunId,
    dataset: &str,
) -> Result<RunReader, CallError> {
    let mut reader = context
        .runs()
        .reader(run_id)
        .await
        .map_err(|error| run_error(error, context.identity()))?;
    reader
        .read(Declarations::read)
        .await
        .map_err(|error| run_error(error, context.identity()))?
        .readable(dataset)?;
    Ok(reader)
}
