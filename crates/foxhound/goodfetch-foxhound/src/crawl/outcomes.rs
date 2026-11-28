//! A crawl's request outcomes, current failed requests and failure retry
//! (ADRs 0102 and 0117; issue 1495): [`ReadOutcomes`] reads `run.outcomes`
//! page after page, [`ReadFailedRequests`] reads `run.failed_requests`, and
//! [`RetryFailed`] re-admits them with `run.retry_failed`.

use std::future::IntoFuture;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use goodfetch_collie_client::{ClientConnection, Error, ErrorCode, EventStream, Reply};
use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::outcomes::{
    CrawlRequestOutcome, CrawlRequestOutcomes, FailedRequests, Now, OutcomeKind, OutcomesAfter,
    RUN_FAILED_REQUESTS, RUN_OUTCOMES, RUN_RETRY_FAILED, RetriedFailed, RunFailedRequestsParams,
    RunOutcomesParams, RunRetryFailedParams,
};
use goodfetch_protocol::run::RunId;
use serde_json::Map;

use super::records::paged_pages;
use crate::settings::Settings;

/// The read of a crawl's outcomes that
/// [`CrawlRun::outcomes`](crate::CrawlRun::outcomes) returns: a log of
/// endings, not current state, read page after page from `run.outcomes` once
/// first polled. Dropping it cancels the page being read and leaves the Run
/// as it is.
#[derive(Debug)]
#[must_use = "a read sends nothing until it is polled"]
pub struct ReadOutcomes {
    connection: ClientConnection,
    params: RunOutcomesParams,
    read: Option<EventStream<CrawlRequestOutcome>>,
}

impl ReadOutcomes {
    /// Reads outcomes of `run_id` in commit order from the first outcome.
    pub(crate) fn new(connection: ClientConnection, run_id: RunId) -> Self {
        Self {
            connection,
            params: RunOutcomesParams {
                run_id,
                kinds: None,
                labels: None,
                codes: None,
                after: None,
            },
            read: None,
        }
    }

    /// Filters outcomes in the engine to only those of `kinds`.
    pub fn kinds(mut self, kinds: impl IntoIterator<Item = OutcomeKind>) -> Self {
        self.params.kinds = Some(kinds.into_iter().collect());
        self
    }

    /// Filters outcomes in the engine to only requests of `labels`.
    pub fn labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.params.labels = Some(labels.into_iter().map(Into::into).collect());
        self
    }

    /// Filters outcomes in the engine to only requests one of whose failed
    /// attempts ended with one of `codes`.
    pub fn codes(mut self, codes: impl IntoIterator<Item = ErrorCode>) -> Self {
        self.params.codes = Some(codes.into_iter().map(|c| c.as_str().to_owned()).collect());
        self
    }

    /// Reads outcomes committed after position `after`.
    pub fn after(mut self, after: u64) -> Self {
        self.params.after = Some(OutcomesAfter::Position(after));
        self
    }

    /// Reads only outcomes committed after this read begins (`after: "now"`).
    pub fn after_now(mut self) -> Self {
        self.params.after = Some(OutcomesAfter::Now(Now::Now));
        self
    }

    /// The next outcome, or `None` once the read has ended; the first call
    /// asks the first page.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future before it completes loses no outcome,
    /// and the page being read stays asked.
    pub async fn next(&mut self) -> Option<Result<CrawlRequestOutcome, Error>> {
        StreamExt::next(self).await
    }

    /// Sends the read, turning it into a paged stream.
    pub(crate) fn sent(self) -> EventStream<CrawlRequestOutcome> {
        if let Some(read) = self.read {
            return read;
        }
        paged_pages(
            self.connection,
            RUN_OUTCOMES.name,
            self.params,
            |next_params: &mut RunOutcomesParams, page: CrawlRequestOutcomes| {
                next_params.after = Some(OutcomesAfter::Position(page.next));
                (page.outcomes, page.ended)
            },
        )
    }
}

impl Stream for ReadOutcomes {
    type Item = Result<CrawlRequestOutcome, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let outcomes = self.get_mut();
        let read = outcomes.read.get_or_insert_with(|| {
            let unsent = Self {
                connection: outcomes.connection.clone(),
                params: outcomes.params.clone(),
                read: None,
            };
            unsent.sent()
        });
        read.poll_next_unpin(context)
    }
}

/// The call that reads the requests whose latest outcome is `failed` (and
/// optionally `blocked`), built by
/// [`CrawlRun::failed_requests`](crate::CrawlRun::failed_requests): nothing
/// is sent until it is awaited.
#[derive(Debug)]
#[must_use = "a read sends nothing until it is awaited"]
pub struct ReadFailedRequests {
    connection: ClientConnection,
    params: RunFailedRequestsParams,
    timeout: Option<Duration>,
}

impl ReadFailedRequests {
    /// Prepares a read of the failed requests of `run_id`.
    pub(crate) fn new(connection: ClientConnection, run_id: RunId) -> Self {
        Self {
            connection,
            params: RunFailedRequestsParams {
                run_id,
                blocked: false,
            },
            timeout: None,
        }
    }

    /// When `true`, also lists requests whose latest outcome is `blocked`.
    pub fn blocked(mut self, blocked: bool) -> Self {
        self.params.blocked = blocked;
        self
    }

    /// Bounds the wait for `run.failed_requests` to answer.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl IntoFuture for ReadFailedRequests {
    type Output = Result<Vec<CrawlRequestOutcome>, Error>;
    type IntoFuture = Reply<Vec<CrawlRequestOutcome>>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            params,
            timeout,
        } = self;
        Box::pin(async move {
            let mut call = connection.call::<FailedRequests>(RUN_FAILED_REQUESTS.name, &params);
            if let Some(timeout) = timeout {
                call = call.timeout(timeout);
            }
            Ok(call.await?.requests)
        })
    }
}

/// The call that re-admits every request whose latest outcome is `failed`
/// (and optionally `blocked`), built by
/// [`CrawlRun::retry_failed`](crate::CrawlRun::retry_failed): nothing is sent
/// until it is awaited.
#[derive(Debug)]
#[must_use = "a retry sends nothing until it is awaited"]
pub struct RetryFailed {
    connection: ClientConnection,
    params: RunRetryFailedParams,
    settings: Settings,
    timeout: Option<Duration>,
}

impl RetryFailed {
    /// Prepares a retry of the failed requests of `run_id`.
    pub(crate) fn new(connection: ClientConnection, run_id: RunId) -> Self {
        Self {
            connection,
            params: RunRetryFailedParams {
                run_id,
                blocked: false,
                settings: Map::new(),
                project: None,
            },
            settings: Settings::new(),
            timeout: None,
        }
    }

    /// When `true`, also re-admits requests whose latest outcome is `blocked`.
    pub fn blocked(mut self, blocked: bool) -> Self {
        self.params.blocked = blocked;
        self
    }

    /// Sets the Run's settings from the retry on, read when the retry returns
    /// an ended Run to `running` (ADRs 0083 and 0102).
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Bounds the wait for `run.retry_failed` to answer.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl IntoFuture for RetryFailed {
    type Output = Result<u64, Error>;
    type IntoFuture = Reply<u64>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            mut params,
            settings,
            timeout,
        } = self;
        Box::pin(async move {
            params.settings = settings.within(SettingScope::Run)?;
            params.project = connection.project().cloned();
            let mut call = connection.call::<RetriedFailed>(RUN_RETRY_FAILED.name, &params);
            if let Some(timeout) = timeout {
                call = call.timeout(timeout);
            }
            Ok(call.await?.readmitted)
        })
    }
}
