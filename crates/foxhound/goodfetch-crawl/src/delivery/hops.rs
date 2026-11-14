//! The redirect hops of a crawl's pages (ADR 0142): a hop's check against
//! the visited set, and the record of a redirected chain's final URL, are
//! made by the delivery loop, the actor that owns the crawl's Run state,
//! never in a job a page's task queues with the run-state writer itself.
//! A page's task asks in one message on the loop's bounded mailbox and
//! waits for the answer: one round trip a hop, counted as
//! `F3.hop_round_trips`, and one more a redirected chain, for its end,
//! counted as `F3.hop_round_trips.end`. The end's record does not fold
//! into the attempt's outcome: it is made before the handler runs, which a
//! chain that reached another request's page never does (ADR 0120).
//!
//! The visited set is still the run file's, so the loop answers from the
//! file: it takes the question's place in the run-state writer's queue, in
//! its own order among the outcomes it queues, and the page's task waits
//! for that job, which the loop does not. Neither answer promises anything
//! outside, so neither is synced: the attempt's outcome syncs them
//! (ADR 0138's note of #1926).
//!
//! A chain runs on its page's task, so a question comes while a page runs,
//! when the loop is at its mailboxes.

use std::pin::Pin;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_state::{RunStateError, Syncing};
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{ChainHop, HopAnswer, HopChecks};
use goodfetch_frontier::{Hop, Invariants};
use tokio::sync::{mpsc, oneshot};

/// How many questions wait in the loop's mailbox before the next page's
/// task waits to send its own.
const MAILBOX: usize = 64;

/// What a page's task asks of a hop.
#[derive(Clone, Copy)]
enum Asks {
    /// Where the hop leads its request.
    Check,
    /// That its chain ended on it.
    Reached,
}

/// The loop's answer: the job it queued for the question, which the
/// page's task waits for.
type Queued = Pin<Box<dyn Future<Output = Result<Hop, RunStateError>> + Send>>;

/// A page's task's one message to the loop about a hop.
pub(super) struct Asked {
    hop: ChainHop,
    asks: Asks,
    answer: oneshot::Sender<Queued>,
}

/// The pages' side of the mailbox, which the Run's hand-over carries down
/// to each chain.
struct Asking {
    work: CountedWork,
    mailbox: mpsc::Sender<Box<Asked>>,
}

impl Asking {
    /// Asks the loop `asks` of `hop` and waits for its answer: one round
    /// trip. `Closed` once the loop ended.
    async fn ask(&self, hop: ChainHop, asks: Asks) -> Result<Hop, RunStateError> {
        let (answer, answered) = oneshot::channel();
        self.work.count(match asks {
            Asks::Check => Row::HopRoundTrips,
            Asks::Reached => Row::HopEndRoundTrips,
        });
        self.mailbox
            .send(Box::new(Asked { hop, asks, answer }))
            .await
            .map_err(|_| RunStateError::Closed)?;
        answered.await.map_err(|_| RunStateError::Closed)?.await
    }
}

impl HopChecks for Asking {
    fn check(&self, hop: ChainHop) -> HopAnswer<'_> {
        Box::pin(self.ask(hop, Asks::Check))
    }

    fn reached(&self, hop: ChainHop) -> HopAnswer<'_> {
        Box::pin(self.ask(hop, Asks::Reached))
    }
}

/// The loop's side of the mailbox, with what a hop is judged within: the
/// Run's domain policy and its admission invariants, built once (#1896).
pub(super) struct Hops {
    asked: mpsc::Receiver<Box<Asked>>,
    egress: Arc<Policy>,
    invariants: Arc<Invariants>,
}

impl Hops {
    /// The mailbox of `run`'s crawl, whose hops are judged under `egress`
    /// within `invariants`, and the handle its pages' chains ask through.
    pub(super) fn of(
        run: &Run,
        egress: &Arc<Policy>,
        invariants: Arc<Invariants>,
    ) -> (Self, Arc<dyn HopChecks>) {
        let (mailbox, asked) = mpsc::channel(MAILBOX);
        let asking = Asking {
            work: run.metrics().counted_work().clone(),
            mailbox,
        };
        (
            Self {
                asked,
                egress: Arc::clone(egress),
                invariants,
            },
            Arc::new(asking),
        )
    }

    /// The next question a page's task asks; none once no chain can ask.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future takes no question out of the mailbox.
    pub(super) async fn asked(&mut self) -> Option<Box<Asked>> {
        self.asked.recv().await
    }

    /// Answers `asked` with its job, queued with `run`'s run-state writer
    /// now: the hop's check, counted as `F3.transactions.hop`, or the
    /// record of its chain's end.
    pub(super) async fn answer(&self, run: &Run, asked: Box<Asked>) {
        let Asked { hop, asks, answer } = *asked;
        let queued: Queued = match run.state().reserve().await {
            Ok(place) => {
                let unsynced = Syncing::Deferred { charged: 0 };
                match asks {
                    Asks::Check => {
                        run.metrics().counted_work().count(Row::HopTransactions);
                        let (egress, invariants) =
                            (Arc::clone(&self.egress), Arc::clone(&self.invariants));
                        Box::pin(place.write_with(unsynced, move |connection| {
                            goodfetch_frontier::admit_hop(
                                connection,
                                &invariants,
                                &egress,
                                &hop.crawl_request,
                                (&hop.url, &hop.method, hop.body.as_deref()),
                            )
                        }))
                    }
                    Asks::Reached => Box::pin(place.write_with(unsynced, move |connection| {
                        goodfetch_frontier::redirect_reached(
                            connection,
                            &hop.crawl_request,
                            &hop.url,
                            &hop.method,
                            hop.body.as_deref(),
                        )
                    })),
                }
            }
            Err(error) => Box::pin(std::future::ready(Err(error))),
        };
        // Its task is gone when the Run's cancellation took it.
        drop(answer.send(queued));
    }
}
