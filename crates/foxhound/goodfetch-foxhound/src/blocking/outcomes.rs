//! A crawl's request outcomes, current failed requests and failure retry for
//! a program that does not use async (ADRs 0081, 0102 and 0117; issue 1495):
//! the blocking twins of [`crate::ReadOutcomes`],
//! [`crate::ReadFailedRequests`] and [`crate::RetryFailed`].

use std::future::IntoFuture as _;
use std::time::Duration;

use goodfetch_collie_client::blocking::{self, EventIter};
use goodfetch_collie_client::{Error, ErrorCode};
use goodfetch_protocol::foxhound::outcomes::{CrawlRequestOutcome, OutcomeKind};

use crate::settings::Settings;

/// The read of a crawl's outcomes, as [`crate::ReadOutcomes`], as an iterator
/// that blocks the calling thread until the next one. Sent on first iteration;
/// dropping it cancels the page being read.
#[derive(Debug)]
#[must_use = "a read sends nothing until it is iterated"]
pub struct ReadOutcomes {
    unsent: Option<crate::ReadOutcomes>,
    sent: Option<EventIter<CrawlRequestOutcome>>,
    connection: blocking::ClientConnection,
}

impl ReadOutcomes {
    pub(super) fn new(read: crate::ReadOutcomes, connection: blocking::ClientConnection) -> Self {
        Self {
            unsent: Some(read),
            sent: None,
            connection,
        }
    }

    /// Filters outcomes in the engine to only those of `kinds`.
    pub fn kinds(mut self, kinds: impl IntoIterator<Item = OutcomeKind>) -> Self {
        if let Some(unsent) = self.unsent.take() {
            self.unsent = Some(unsent.kinds(kinds));
        }
        self
    }

    /// Filters outcomes in the engine to only requests of `labels`.
    pub fn labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        if let Some(unsent) = self.unsent.take() {
            self.unsent = Some(unsent.labels(labels));
        }
        self
    }

    /// Filters outcomes in the engine to only requests one of whose failed
    /// attempts ended with one of `codes`.
    pub fn codes(mut self, codes: impl IntoIterator<Item = ErrorCode>) -> Self {
        if let Some(unsent) = self.unsent.take() {
            self.unsent = Some(unsent.codes(codes));
        }
        self
    }

    /// Reads outcomes committed after position `after`.
    pub fn after(mut self, after: u64) -> Self {
        if let Some(unsent) = self.unsent.take() {
            self.unsent = Some(unsent.after(after));
        }
        self
    }

    /// Reads only outcomes committed after this read begins (`after: "now"`).
    pub fn after_now(mut self) -> Self {
        if let Some(unsent) = self.unsent.take() {
            self.unsent = Some(unsent.after_now());
        }
        self
    }
}

impl Iterator for ReadOutcomes {
    type Item = Result<CrawlRequestOutcome, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let sent = self.sent.get_or_insert_with(|| {
            let unsent = self.unsent.take().expect("invariant: sent once");
            self.connection.iter(unsent.sent())
        });
        sent.next()
    }
}

/// The call that reads the requests whose latest outcome is `failed` (and
/// optionally `blocked`), as [`crate::ReadFailedRequests`], which `send()`
/// sends.
#[derive(Debug)]
#[must_use = "a read sends nothing until `send()` is called"]
pub struct ReadFailedRequests {
    inner: crate::ReadFailedRequests,
    connection: blocking::ClientConnection,
}

impl ReadFailedRequests {
    pub(super) fn new(
        inner: crate::ReadFailedRequests,
        connection: blocking::ClientConnection,
    ) -> Self {
        Self { inner, connection }
    }

    /// When `true`, also lists requests whose latest outcome is `blocked`.
    pub fn blocked(mut self, blocked: bool) -> Self {
        self.inner = self.inner.blocked(blocked);
        self
    }

    /// Bounds the wait for `run.failed_requests` to answer.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    /// Sends `run.failed_requests` and blocks until it answers.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::ReadFailedRequests`].
    pub fn send(self) -> Result<Vec<CrawlRequestOutcome>, Error> {
        self.connection.wait(self.inner.into_future())
    }
}

/// The call that re-admits every request whose latest outcome is `failed`
/// (and optionally `blocked`), as [`crate::RetryFailed`], which `send()`
/// sends.
#[derive(Debug)]
#[must_use = "a retry sends nothing until `send()` is called"]
pub struct RetryFailed {
    inner: crate::RetryFailed,
    connection: blocking::ClientConnection,
}

impl RetryFailed {
    pub(super) fn new(inner: crate::RetryFailed, connection: blocking::ClientConnection) -> Self {
        Self { inner, connection }
    }

    /// When `true`, also re-admits requests whose latest outcome is `blocked`.
    pub fn blocked(mut self, blocked: bool) -> Self {
        self.inner = self.inner.blocked(blocked);
        self
    }

    /// Sets the Run's settings from the retry on, read when the retry returns
    /// an ended Run to `running`.
    pub fn settings(mut self, settings: Settings) -> Self {
        self.inner = self.inner.settings(settings);
        self
    }

    /// Bounds the wait for `run.retry_failed` to answer.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    /// Sends `run.retry_failed` and blocks until it answers.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::RetryFailed`].
    pub fn send(self) -> Result<u64, Error> {
        self.connection.wait(self.inner.into_future())
    }
}

impl super::CrawlRun {
    /// A log of endings, not current state: a re-admitted request's new
    /// outcome supersedes the old by `request.id` (ADRs 0102 and 0117).
    ///
    /// Reads a crawl's outcomes from its committed state, in commit order:
    /// from the first, after a position, or with `after: "now"` from the next
    /// commit. Its filters, `kinds`, `labels` and `codes` (an Error code any
    /// of the request's failed attempts failed with), run in the engine.
    /// While the Run is live and nothing new matches, it waits for the next
    /// commit; a page says `ended` once the Run has ended, or has no live
    /// writer, and nothing more matches. A reader never slows the Run. A
    /// request `run.retry_failed` re-admits ends again, and its later outcome
    /// supersedes the earlier one by `request.id`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn outcomes(&self) -> ReadOutcomes {
        ReadOutcomes::new(self.inner.outcomes(), self.connection.clone())
    }

    /// Reads from a crawl's committed state the requests whose latest outcome
    /// is `failed`, and with `blocked` also `blocked`, each with that outcome:
    /// exactly those `run.retry_failed` re-admits with the same option. A skip
    /// is a decision and is never listed.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn failed_requests(&self) -> ReadFailedRequests {
        ReadFailedRequests::new(self.inner.failed_requests(), self.connection.clone())
    }

    /// Re-admits, in the same Run and its Datasets, every request of a crawl
    /// whose latest outcome is `failed`, and with `blocked` also `blocked`:
    /// exactly those `run.failed_requests` lists with the same option. Each
    /// keeps its ID, Label, Payload and depth; its attempts count on, with a
    /// fresh `crawl.retry.max`. A skip is a decision and is never re-admitted.
    /// It answers how many it re-admitted, 0 when there was nothing to retry.
    /// On a crawl this engine runs, the requests join its work at once, unless
    /// the crawl is already ending its Run: the call then waits for that end
    /// and re-admits them as on an ended Run. On an ended Run with requests to
    /// retry, the Run returns to `running` under `settings`, as a resume does,
    /// its Verdict and Completeness given anew when it ends, and the calling
    /// Connection serves its handlers. A Run with no writer that has not ended
    /// fails with `run_interrupted`: resume it first. A request to retry whose
    /// Label the crawl no longer declares, which a resume may drop, fails the
    /// call with `label_unknown`, naming that Label and those declared, and
    /// nothing is re-admitted: on an ended Run, against the Labels its latest
    /// start declared, before the Run returns to `running`. It is not sent
    /// again after a lost Connection, which fails it with `connection_lost`:
    /// `run.failed_requests` then lists the requests still to retry.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn retry_failed(&self) -> RetryFailed {
        RetryFailed::new(self.inner.retry_failed(), self.connection.clone())
    }
}
