//! The crawl's Run as a handler's context reaches it (ADR 0093): bound to
//! the attempt, so what is written through it is staged with the attempt.

use goodfetch_collie_client::{KeyValueStore, Run};
use goodfetch_protocol::foxhound::crawl::CrawlRequestHandleParams;
use goodfetch_protocol::key_value_store::KeyValueStoreAttempt;
use goodfetch_protocol::run::RunId;

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
}

impl AttemptRun {
    /// `run` bound to the attempt `params` names.
    pub(in crate::crawl) fn new(run: &Run, params: &CrawlRequestHandleParams) -> Self {
        let attempt = KeyValueStoreAttempt {
            crawl_request_id: params.crawl_request.id.clone(),
            lease_epoch: params.lease_epoch,
        };
        Self {
            run_id: params.run_id.clone(),
            key_value_store: run.key_value_store().bound_to(attempt),
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
}
