//! `run.records` (ADRs 0008, 0024 and 0103): a cursor over one Dataset's
//! committed Records, read through the engine's Run reader on a read-only
//! connection of its own, never the Run's writer. It sends each Record in
//! a `$/record` notification only once its Client granted a credit, and
//! holds no read transaction while it waits for one, so a reader that
//! stops reading costs the crawl nothing: the crawl commits as if no
//! reader were there. While this engine's writer works the Run, it waits
//! for the next commit, and while another engine's does, as a reader on
//! the Daemon of a Run an Attached engine crawls, it looks at the run file
//! again every second; it answers once the Run has ended, or has no live
//! writer anywhere, and it sent every Record, or, given `through`, once it
//! sent every Record numbered through it, whether the Run is live or not.

use goodfetch_engine::run::run_error;
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::dataset::{
    RECORD, RUN_RECORDS, RecordRead, RecordsRead, RunRecordsParams,
};

use crate::open::open_dataset;
use crate::records::Pages;

/// The handler of `run.records`.
pub(crate) fn handler() -> Handler {
    Handler::new(&RUN_RECORDS, read_records).streaming()
}

async fn read_records(
    params: RunRecordsParams,
    context: CallContext,
) -> Result<RecordsRead, CallError> {
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = open_dataset(&context, params.run_id.clone(), &params.dataset).await?;
    let stream = context.stream();
    let after = params.after.unwrap_or_default();
    let mut read = RecordsRead {
        count: 0,
        last: after,
    };
    let mut pages = Pages::new(&params.dataset, after, params.through, params.wall);
    loop {
        let live = reader.is_live().await;
        let (page, full, committed) = pages
            .next_records_through(&mut reader)
            .await
            .map_err(failed)?;
        for record in page {
            let _ = stream.spend(1).await;
            let number = record.number;
            let sent = stream
                .notify(
                    RECORD,
                    &RecordRead {
                        id: stream.id().clone(),
                        record,
                    },
                )
                .await;
            if !sent {
                return Ok(read);
            }
            read.count += 1;
            read.last = number;
        }
        if full {
            continue;
        }
        if committed || !live {
            return Ok(read);
        }
        reader.changed().await;
    }
}
