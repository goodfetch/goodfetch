//! Seeds of a running crawl (`run.add_seed`, `run.add_seeds`, ADRs 0024, 0093
//! and 0102; issue 1494): [`AddSeed`] adds one seed and [`AddSeeds`] streams
//! seeds under the engine's `$/credit` grants, yielding each seed's
//! [`Enqueued`] answer in order without buffering unbounded seeds in memory.

use std::collections::VecDeque;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Stream;
use goodfetch_collie_client::{ClientConnection, Error, Feed, FeedEvent};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::foxhound::crawl::Seed as WireSeed;
use goodfetch_protocol::foxhound::frontier::{
    ENQUEUED, Enqueued, EnqueuedBatch, RUN_ADD_SEED, RUN_ADD_SEEDS, RunAddSeedParams,
    RunAddSeedsParams, SEEDS, SeedAttempt, SeedBatch,
};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::run::RunId;

use super::{CrawlId, own_seed};
use crate::Seed;

/// Maximum seeds sent in a single `$/seeds` notification batch, staying well
/// under `SEED_CREDITS` (256) so answers stream back incrementally.
const SEED_BATCH_MAX: usize = 64;

/// Adds one seed to a running crawl (`run.add_seed`), which awaiting sends.
/// Nothing is sent until it is awaited.
///
/// Admitted and answered as soon as the Run state accepts it (written with the
/// next Run-state batch at default durability, or committed before answering
/// at `full`). Through [`AttemptRun::add_seed`](crate::AttemptRun::add_seed),
/// it is staged with the attempt.
#[must_use = "adding a seed sends nothing until it is awaited"]
#[derive(Debug)]
pub struct AddSeed {
    connection: ClientConnection,
    params: Result<RunAddSeedParams, Error>,
    timeout: Option<Duration>,
}

impl AddSeed {
    /// Builds an `add_seed` call for `run_id`, optionally bound to `attempt`.
    pub(crate) fn new(
        connection: ClientConnection,
        run_id: RunId,
        crawl: CrawlId,
        declared: &[String],
        seed: Seed,
        attempt: Option<SeedAttempt>,
    ) -> Self {
        let params = own_seed(crawl, declared, seed).map(|wire_seed| RunAddSeedParams {
            run_id,
            seed: wire_seed,
            attempt,
        });
        Self {
            connection,
            params,
            timeout: None,
        }
    }

    /// Sets a timeout on the `run.add_seed` call.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl IntoFuture for AddSeed {
    type Output = Result<Enqueued, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Enqueued, Error>> + Send + 'static>>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            params,
            timeout,
        } = self;
        Box::pin(async move {
            let params = params?;
            let call = connection.call::<Enqueued>(RUN_ADD_SEED.name, &params);
            let call = match timeout {
                Some(duration) => call.timeout(duration),
                None => call,
            };
            call.await
        })
    }
}

/// Streams seeds to a running crawl (`run.add_seeds`) under the engine's
/// `$/credit` backpressure, yielding each seed's [`Enqueued`] answer in order.
///
/// The `run.add_seeds` call opens when first polled; dropping the stream
/// before it ends cancels the call (`$/cancel`) without blocking, keeping the
/// seeds already answered. Memory stays bounded by the engine's credit window
/// (at most 256 seeds in flight) even over a million-seed generator.
#[must_use = "a seed stream sends nothing until it is polled"]
pub struct AddSeeds {
    connection: ClientConnection,
    run_id: RunId,
    attempt: Option<SeedAttempt>,
    crawl: CrawlId,
    declared: Arc<[String]>,
    source: Pin<Box<dyn Stream<Item = Seed> + Send>>,
    feed: Option<Feed>,
    call_id: Option<i64>,
    credits: usize,
    source_ended: bool,
    end_sent: bool,
    answers: VecDeque<Enqueued>,
    terminal: Option<Result<(), Error>>,
    done: bool,
}

impl fmt::Debug for AddSeeds {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AddSeeds")
            .field("run_id", &self.run_id)
            .field("credits", &self.credits)
            .field("source_ended", &self.source_ended)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl AddSeeds {
    /// Builds a streaming `run.add_seeds` call from `source`, optionally bound
    /// to `attempt`.
    pub(crate) fn new<S>(
        connection: ClientConnection,
        run_id: RunId,
        crawl: CrawlId,
        declared: Arc<[String]>,
        source: S,
        attempt: Option<SeedAttempt>,
    ) -> Self
    where
        S: Stream<Item = Seed> + Send + 'static,
    {
        Self {
            connection,
            run_id,
            attempt,
            crawl,
            declared,
            source: Box::pin(source),
            feed: None,
            call_id: None,
            credits: 0,
            source_ended: false,
            end_sent: false,
            answers: VecDeque::new(),
            terminal: None,
            done: false,
        }
    }

    /// Reads as many seeds from `self.source` as current credits and batch size
    /// permit without blocking once at least one seed is collected, then sends
    /// one `$/seeds` batch. Returns `Err` if a seed fails client-side checks.
    fn pump_source(&mut self, context: &mut Context<'_>) -> Result<(), Error> {
        let Some(id) = self.call_id else {
            return Ok(());
        };
        if self.end_sent {
            return Ok(());
        }
        // Only pull seeds while we hold credits or need to send the final end marker.
        if self.credits == 0 && !self.source_ended {
            return Ok(());
        }
        let max_take = self.credits.min(SEED_BATCH_MAX);
        let mut batch: Vec<WireSeed> = Vec::new();
        while !self.source_ended && batch.len() < max_take {
            match self.source.as_mut().poll_next(context) {
                Poll::Ready(Some(seed)) => {
                    let wire = own_seed(self.crawl, &self.declared, seed)?;
                    batch.push(wire);
                }
                Poll::Ready(None) => {
                    self.source_ended = true;
                    break;
                }
                Poll::Pending => break,
            }
        }
        // Send if we gathered any seeds OR if the source ended and we haven't sent `end: true`.
        if batch.is_empty() && !self.source_ended {
            return Ok(());
        }
        let taken = batch.len();
        let end = self.source_ended;
        self.credits = self.credits.saturating_sub(taken);
        if end {
            self.end_sent = true;
        }
        if let Some(feed) = &self.feed {
            feed.notify(
                SEEDS,
                &SeedBatch {
                    id: RequestId::Number(id),
                    seeds: batch,
                    end,
                },
            )?;
        }
        Ok(())
    }

    /// Handles one incoming notification from the engine (`$/credit` or `$/enqueued`).
    fn handle_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), Error> {
        if method == CREDIT {
            let granted: CreditParams = Feed::decode(params)?;
            self.credits = self.credits.saturating_add(granted.credits as usize);
        } else if method == ENQUEUED {
            let batch: EnqueuedBatch = Feed::decode(params)?;
            self.answers.extend(batch.answers);
        }
        Ok(())
    }
}

impl Stream for AddSeeds {
    type Item = Result<Enqueued, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        if this.feed.is_none() {
            let params = RunAddSeedsParams {
                run_id: this.run_id.clone(),
                attempt: this.attempt.clone(),
            };
            this.feed = Some(this.connection.feed(RUN_ADD_SEEDS.name, &params));
        }

        loop {
            if let Some(answer) = this.answers.pop_front() {
                // Keep pumping credits/batches opportunistically when the answer buffer is low.
                // Surface any client-side source error only after buffered answers drain.
                if this.answers.len() < SEED_BATCH_MAX
                    && this.terminal.is_none()
                    && let Err(error) = this.pump_source(context)
                {
                    this.terminal = Some(Err(error));
                    this.feed = None;
                }
                return Poll::Ready(Some(Ok(answer)));
            }

            if let Some(terminal) = this.terminal.take() {
                this.done = true;
                this.feed = None;
                return match terminal {
                    Ok(()) => Poll::Ready(None),
                    Err(error) => Poll::Ready(Some(Err(error))),
                };
            }

            if let Err(error) = this.pump_source(context) {
                this.done = true;
                this.feed = None;
                return Poll::Ready(Some(Err(error)));
            }

            let Some(feed) = this.feed.as_mut() else {
                this.done = true;
                return Poll::Ready(None);
            };
            match feed.poll_event(context) {
                Poll::Ready(FeedEvent::Sent(id)) => {
                    this.call_id = Some(id);
                }
                Poll::Ready(FeedEvent::Notified { method, params }) => {
                    if let Err(error) = this.handle_notification(&method, params) {
                        this.done = true;
                        this.feed = None;
                        return Poll::Ready(Some(Err(error)));
                    }
                }
                Poll::Ready(FeedEvent::Finished(result)) => {
                    this.terminal = Some(result.map(|_| ()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
