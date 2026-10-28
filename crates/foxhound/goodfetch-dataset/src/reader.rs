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
//! writer anywhere, and it sent every Record.

use goodfetch_engine::run::run_error;
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::dataset::{
    RECORD, RUN_RECORDS, RecordRead, RecordsRead, RunRecordsParams,
};

use crate::declaration::Datasets;
use crate::records::{self, Query};

/// The most Records one read of the run file takes.
const PAGE: usize = 256;

/// The handler of `run.records`.
pub(crate) fn handler() -> Handler {
    Handler::new(&RUN_RECORDS, read_records).streaming()
}

async fn read_records(
    params: RunRecordsParams,
    context: CallContext,
) -> Result<RecordsRead, CallError> {
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    reader
        .read(Datasets::read)
        .await
        .map_err(failed)?
        .readable(&params.dataset)?;
    let stream = context.stream();
    let mut read = RecordsRead {
        count: 0,
        last: params.after.unwrap_or_default(),
    };
    loop {
        let live = reader.is_live().await;
        let (dataset, after) = (params.dataset.clone(), read.last);
        let page = reader
            .read(move |connection| {
                records::read(
                    connection,
                    Query {
                        dataset: &dataset,
                        after,
                        through: None,
                        wall: params.wall,
                    },
                    PAGE,
                )
            })
            .await
            .map_err(failed)?;
        let full = page.len() == PAGE;
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
        if !live {
            return Ok(read);
        }
        reader.changed().await;
    }
}
