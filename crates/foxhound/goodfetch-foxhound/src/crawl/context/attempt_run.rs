//! The crawl's Run as a handler's context reaches it (ADR 0093): bound to
//! the attempt, so what is written through it is staged with the attempt,
//! and what is read from it ends at what had committed (ADR 0103).

use goodfetch_collie_client::{ClientConnection, KeyValueStore, Run};
use goodfetch_protocol::foxhound::crawl::CrawlRequestHandleParams;
use goodfetch_protocol::key_value_store::KeyValueStoreAttempt;
use goodfetch_protocol::run::RunId;

use crate::crawl::Dataset;
use crate::crawl::records::{Reach, ReadRecords, ReadRejectedRecords};

/// A crawl's Run bound to one attempt of its handler:
/// [`CrawlContext::run`](crate::CrawlContext::run).
///
/// What is written through it commits with the attempt's Records and
/// enqueues, or is discarded with them. The program's own Run value is not
/// bound: a handler that captures it commits at once, on every attempt that
/// runs it, and the write stays though the attempt fails.
#[derive(Clone, Debug)]
pub struct AttemptRun {
    run_id: RunId,
    key_value_store: KeyValueStore,
    connection: ClientConnection,
}

impl AttemptRun {
    /// `run` bound to the attempt `params` names, read on `connection`.
    pub(in crate::crawl) fn new(
        run: &Run,
        connection: ClientConnection,
        params: &CrawlRequestHandleParams,
    ) -> Self {
        let attempt = KeyValueStoreAttempt {
            crawl_request_id: params.crawl_request.id.clone(),
            lease_epoch: params.lease_epoch,
        };
        Self {
            run_id: params.run_id.clone(),
            key_value_store: run.key_value_store().bound_to(attempt),
            connection,
        }
    }

    /// The Run's ID, read without a round trip.
    pub fn id(&self) -> &RunId {
        &self.run_id
    }

    /// The Run's Key-value store, its writes staged with the attempt. A
    /// read sees the attempt's own writes first; other attempts and the
    /// program's own Run value see them once they commit. Once the attempt
    /// ends, each call fails with `attempt_ended`.
    pub fn key_value_store(&self) -> KeyValueStore {
        self.key_value_store.clone()
    }

    /// Reads the Records of `dataset` committed when the read is first
    /// polled, as [`CrawlRun::records`](crate::CrawlRun::records) reads
    /// them, then ends: the Run cannot end while the handler holds its
    /// request, so a read that followed it would wait for the handler's
    /// timeout (ADR 0103). It asks `dataset.page` for the last Record
    /// committed, then reads through it. The attempt's own Records are not
    /// among them: they commit once the handler returns.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// An item fails as [`CrawlRun::records`](crate::CrawlRun::records)'s
    /// does, after which the stream ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the read, and leaves the Run as it is.
    pub fn records<I>(&self, dataset: &Dataset<I>) -> ReadRecords<I> {
        ReadRecords::new(
            self.connection.clone(),
            self.run_id.clone(),
            &dataset.name,
            Reach::Committed,
        )
    }

    /// Reads the entries of `dataset`'s rejection table committed when the
    /// read is first polled, as
    /// [`CrawlRun::rejected_records`](crate::CrawlRun::rejected_records)
    /// reads them, then ends, as [`AttemptRun::records`] does. It asks
    /// `dataset.list` how many entries the table holds, then reads through
    /// the last.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// An item fails as
    /// [`CrawlRun::rejected_records`](crate::CrawlRun::rejected_records)'s
    /// does, after which the stream ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the page being read, and leaves the Run
    /// as it is.
    pub fn rejected_records<I>(&self, dataset: &Dataset<I>) -> ReadRejectedRecords {
        ReadRejectedRecords::new(
            self.connection.clone(),
            self.run_id.clone(),
            &dataset.name,
            Reach::Committed,
        )
    }
}
