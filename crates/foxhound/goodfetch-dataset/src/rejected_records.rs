//! `run.rejected_records` (ADRs 0102 and 0103): one Dataset's rejection
//! table, read from the crawl's committed state through the engine's Run
//! reader, never its writer, so a reader that stops reading never slows
//! the crawl, and it works on a Run no program holds. It answers a page
//! of the entries after a number, and waits for the next commit while the
//! Run is live and none follows. A Dataset the crawl does not declare is
//! refused with `dataset_unknown`, as in a Run no crawl wrote.

use goodfetch_engine::run::{run_error, through_owner};
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::rejections::{
    REJECTED_RECORDS_PAGE, RUN_REJECTED_RECORDS, RejectedRecords, RunRejectedRecordsParams,
};

use crate::open::open_dataset;
use crate::rejections::{self, Page, RequestUrl};

/// The handler of `run.rejected_records`, reading each entry's URL through
/// `request_url`.
pub(crate) fn handler(request_url: RequestUrl) -> Handler {
    Handler::new(&RUN_REJECTED_RECORDS, move |params, context| {
        rejected_records(params, context, request_url)
    })
}

async fn rejected_records(
    params: RunRejectedRecordsParams,
    context: CallContext,
    request_url: RequestUrl,
) -> Result<RejectedRecords, CallError> {
    if let Some(answer) =
        through_owner(&context, &RUN_REJECTED_RECORDS, &params.run_id, &params).await?
    {
        return Ok(answer);
    }
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = open_dataset(&context, params.run_id.clone(), &params.dataset).await?;
    let dataset = params.dataset;
    let mut after = params.after.unwrap_or_default();
    loop {
        let live = reader.is_live().await;
        let read = dataset.clone();
        let Page { records, next } = reader
            .read(move |connection| rejections::page(connection, &read, after, request_url))
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
