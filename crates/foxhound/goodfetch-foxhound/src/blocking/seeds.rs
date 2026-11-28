//! Seeds of a running crawl for a program that does not use async (ADRs 0024,
//! 0081, 0093 and 0102; issue 1494): the blocking twins of [`crate::AddSeed`]
//! and [`crate::AddSeeds`].

use std::future::IntoFuture as _;
use std::time::Duration;

use futures_util::StreamExt as _;
use goodfetch_collie_client::Error;
use goodfetch_collie_client::blocking;
use goodfetch_protocol::foxhound::frontier::Enqueued;

/// Adds one seed to a running crawl, as [`crate::AddSeed`], whose
/// [`send`](Self::send) blocks until the Run state accepts it.
#[derive(Debug)]
#[must_use = "adding a seed sends nothing until `send()` is called"]
pub struct AddSeed {
    inner: crate::AddSeed,
    connection: blocking::ClientConnection,
}

impl AddSeed {
    pub(super) fn new(inner: crate::AddSeed, connection: blocking::ClientConnection) -> Self {
        Self { inner, connection }
    }

    /// Sets a timeout on the `run.add_seed` call.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    /// Adds one seed to a running crawl, `run.addSeed`, admitted and answered
    /// as soon as the Run state accepts it (written with the next Run-state
    /// batch at the default durability, or committed before answering at
    /// `full`), and answers as an enqueue does: added, with its crawl request's
    /// ID and its URL as parsed, or not added, with its reason (`duplicate`,
    /// `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is
    /// `invalid_url`. An idle `crawl.keep_alive` crawl takes it and goes on.
    /// Through a handler's context, naming its `attempt`, it is staged with
    /// that attempt: once the attempt ended it fails with `attempt_ended`. A
    /// Run this engine does not run as a live crawl, or a crawl already ending
    /// its Run, fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// As [`crate::AddSeed`].
    pub fn send(self) -> Result<Enqueued, Error> {
        let Self { inner, connection } = self;
        connection.wait(inner.into_future())
    }
}

/// Streams seeds to a running crawl, as [`crate::AddSeeds`], as an iterator
/// that blocks the calling thread until each seed's [`Enqueued`] answer
/// arrives in order. Dropping it cancels the call without blocking.
#[derive(Debug)]
#[must_use = "a seed stream sends nothing until it is iterated"]
pub struct AddSeeds {
    inner: Option<crate::AddSeeds>,
    connection: blocking::ClientConnection,
}

impl AddSeeds {
    pub(super) fn new(inner: crate::AddSeeds, connection: blocking::ClientConnection) -> Self {
        Self {
            inner: Some(inner),
            connection,
        }
    }
}

impl Iterator for AddSeeds {
    type Item = Result<Enqueued, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut inner = self.inner.take()?;
        let (inner, item) = match self.connection.wait(async move {
            let item = inner.next().await;
            Ok((inner, item))
        }) {
            Ok(pair) => pair,
            Err(error) => return Some(Err(error)),
        };
        if item.is_some() {
            self.inner = Some(inner);
        }
        item
    }
}
