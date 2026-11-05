//! `run.rejected_records` (ADRs 0102 and 0103): one Dataset's rejection
//! table, read from the crawl's committed state through the engine's Run
//! reader, never its writer, so a reader that stops reading never slows
//! the crawl, and it works on a Run no program holds. It answers a page
//! of the entries after a number, and waits for the next commit while the
//! Run is live and none follows. A Dataset the crawl does not declare is
//! refused with `dataset_unknown`.

use goodfetch_engine::run::{run_error, through_owner};
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::rejections::{
    REJECTED_RECORDS_PAGE, RUN_REJECTED_RECORDS, RejectedRecords, RunRejectedRecordsParams,
};

use rusqlite::{Connection, OptionalExtension as _};

use crate::declaration::Declarations;
use crate::rejections::{self, Page};

/// The handler of `run.rejected_records`.
pub(crate) fn handler() -> Handler {
    Handler::new(&RUN_REJECTED_RECORDS, rejected_records)
}

async fn rejected_records(
    params: RunRejectedRecordsParams,
    context: CallContext,
) -> Result<RejectedRecords, CallError> {
    if let Some(answer) =
        through_owner(&context, &RUN_REJECTED_RECORDS, &params.run_id, &params).await?
    {
        return Ok(answer);
    }
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    let dataset = params.dataset;
    let known = dataset.clone();
    reader
        .read(move |connection| {
            if has_rejection_table(connection)? {
                Declarations::read(connection)
            } else {
                Ok(Declarations::default())
            }
        })
        .await
        .map_err(failed)?
        .readable(&known)?;
    let mut after = params.after.unwrap_or_default();
    loop {
        let live = reader.is_live().await;
        let read = dataset.clone();
        let Page { records, next } = reader
            .read(move |connection| rejections::page(connection, &read, after))
            .await
            .map_err(failed)?;
        let full = records.len() == REJECTED_RECORDS_PAGE;
        if !records.is_empty() || !live {
            return Ok(RejectedRecords {
                records,
                next,
                ended: !live && !full,
            });
        }
        after = next;
        reader.changed().await;
    }
}

/// Whether the run file holds rejection tables: a Run no crawl wrote has
/// none, and declares no Dataset.
fn has_rejection_table(connection: &Connection) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare_cached(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'rejected_records'",
        )
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some())
}
