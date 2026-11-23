//! A Page's owner (ADR 0146): one task, under its Session's owner, that
//! takes what is asked of its Page and runs the work that waits on the
//! browser as its own child tasks, each answering its caller, so that its
//! loop never awaits the browser or the program. Spec #2256 moves a Page's
//! operations and state into it ticket by ticket; a click's pointer input
//! is the first, sent while its caller holds the page's turn, so gestures
//! keep their arrival order (ADR 0085).
//!
//! A panic of one of its tasks closes its Page alone (ADR 0148): the owner
//! keeps `engine_panicked`, naming the Page, which each ask in flight and
//! each later one raises, and which the Session's watch of the Page awaits
//! to close it. A panic of its loop itself is told to its Session's owner.

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::tasks::{TaskOwner, TaskStarter};
use goodfetch_protocol::page::PageId;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::Instrument as _;

use crate::cdp::CdpPage;
use crate::errors::{Call, page_panicked};

/// How many asks wait for a Page's owner at most: a caller past them waits
/// for room.
const ASKS: usize = 32;

/// What a Page's calls hold of its owner, and ask it through; its clones
/// share one owner.
#[derive(Clone)]
pub(crate) struct PageOwner {
    asks: mpsc::Sender<Ask>,
    /// What an ask raises once the owner has gone: the page's `closed`.
    closed: Arc<CallError>,
    /// `engine_panicked`, once a panic of one of its tasks closed it.
    panicked: watch::Receiver<Option<CallError>>,
    /// Where each ask is counted, its Run's (ADR 0140's
    /// `C3.owner_round_trips`).
    work: CountedWork,
}

/// One thing asked of a Page's owner, with where its answer goes.
enum Ask {
    /// A click's pointer input at (`x`, `y`), sent on `page` for `call`,
    /// in `span`, the caller's.
    PointerClick {
        page: CdpPage,
        x: f64,
        y: f64,
        call: Call,
        span: tracing::Span,
        answer: oneshot::Sender<Result<(), CallError>>,
    },
}

/// Where the owner's loop keeps what a panic of one of its tasks raises:
/// `engine_panicked` for the Page `page_id`, counted on `metrics`, its
/// Run's.
struct Panicked {
    told: watch::Sender<Option<CallError>>,
    page_id: PageId,
    metrics: Metrics,
}

impl PageOwner {
    /// Starts the owner of the Page `page_id` as a child of `session`, its
    /// Session's owner, its loop in `span`, its Run's, counting on
    /// `metrics`, its Run's; once it has gone, each ask raises `closed`, or
    /// `engine_panicked` after a panic of one of its tasks. It ends once its
    /// Session's owner is cancelled, once one of its tasks panicked, or
    /// once every clone of what this answers has gone, after its child
    /// tasks.
    pub(crate) fn start(
        session: &TaskStarter,
        span: &tracing::Span,
        page_id: &PageId,
        closed: CallError,
        metrics: &Metrics,
    ) -> Self {
        Self::serving(session.child(), session, span, page_id, closed, metrics)
    }

    /// The owner [`PageOwner::start`] starts, whose tasks `owner` owns.
    fn serving(
        owner: TaskOwner,
        session: &TaskStarter,
        span: &tracing::Span,
        page_id: &PageId,
        closed: CallError,
        metrics: &Metrics,
    ) -> Self {
        let (asks, mailbox) = mpsc::channel(ASKS);
        let (told, panicked) = watch::channel(None);
        let panic = Panicked {
            told,
            page_id: page_id.clone(),
            metrics: metrics.clone(),
        };
        session.spawn(serve(owner, mailbox, panic).instrument(span.clone()));
        Self {
            asks,
            closed: Arc::new(closed),
            panicked,
            work: metrics.counted_work().clone(),
        }
    }

    /// Clicks at (`x`, `y`) in the viewport of `page`, through the owner:
    /// [`CdpPage::pointer_click`] for `call`, as a child task of the owner,
    /// in the caller's span. One round trip to the owner, counted on
    /// `C3.owner_round_trips`.
    ///
    /// # Errors
    ///
    /// What the click raises; once the owner has gone, `closed`, or
    /// `engine_panicked` when a panic of one of its tasks closed it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops the click: none of its later events is
    /// sent.
    pub(crate) async fn pointer_click(
        &self,
        page: CdpPage,
        x: f64,
        y: f64,
        call: &Call,
    ) -> Result<(), CallError> {
        self.work.count(Row::ActionOwnerRoundTrips);
        let (answer, answered) = oneshot::channel();
        let ask = Ask::PointerClick {
            page,
            x,
            y,
            call: call.clone(),
            span: tracing::Span::current(),
            answer,
        };
        if self.asks.send(ask).await.is_err() {
            return Err(self.gone().await);
        }
        match answered.await {
            Ok(clicked) => clicked,
            Err(_) => Err(self.gone().await),
        }
    }

    /// Waits until a panic of one of its tasks closed it, and answers the
    /// `engine_panicked` that panic raises; never answers when none does.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn panicked(&self) -> CallError {
        let mut panicked = self.panicked.clone();
        if let Ok(told) = panicked.wait_for(Option::is_some).await
            && let Some(error) = told.as_ref()
        {
            return error.clone();
        }
        std::future::pending().await
    }

    /// What an ask whose owner has gone raises, once its loop ended:
    /// `engine_panicked` when a panic of one of its tasks ended it, else
    /// `closed`.
    async fn gone(&self) -> CallError {
        // Its loop keeps the panic's error before it ends, and drops its
        // mailbox as it ends.
        self.asks.closed().await;
        self.panicked
            .borrow()
            .clone()
            .unwrap_or_else(|| CallError::clone(&self.closed))
    }
}

/// The owner's loop: it takes each ask in turn and starts its work, never
/// awaiting it, until `owner` is cancelled, one of its tasks panicked, which
/// `panic` keeps, or every [`PageOwner`] has gone, then stops its child
/// tasks.
async fn serve(owner: TaskOwner, mut mailbox: mpsc::Receiver<Ask>, panic: Panicked) {
    let panicked = owner.panicked();
    tokio::pin!(panicked);
    loop {
        let ask = tokio::select! {
            biased;
            () = &mut panicked => {
                let error = page_panicked(
                    &panic.metrics,
                    &panic.page_id,
                    owner.panicked_crash_report(),
                );
                panic.told.send_replace(Some(error));
                break;
            }
            () = owner.cancelled() => break,
            ask = mailbox.recv() => match ask {
                Some(ask) => ask,
                None => break,
            },
        };
        start(&owner, ask);
    }
    owner.stop().await;
}

/// Starts the work `ask` asks for as a child task of `owner`, which
/// answers it, and stops once its caller has gone or `owner` is cancelled.
fn start(owner: &TaskOwner, ask: Ask) {
    match ask {
        Ask::PointerClick {
            page,
            x,
            y,
            call,
            span,
            mut answer,
        } => {
            let stopped = owner.child_token();
            owner.spawn(
                async move {
                    tokio::select! {
                        biased;
                        () = stopped.cancelled() => {}
                        () = answer.closed() => {}
                        clicked = page.pointer_click(x, y, &call) => {
                            let _ = answer.send(clicked);
                        }
                    }
                }
                .instrument(span),
            );
        }
    }
}

#[cfg(test)]
mod tests;
