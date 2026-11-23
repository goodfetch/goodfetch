//! A Page's owner (ADR 0146): one task, under its Session's owner, that
//! takes what is asked of its Page and runs the work that waits on the
//! browser as its own child tasks, each answering its caller, so that its
//! loop never awaits the browser or the program. Spec #2256 moves a Page's
//! operations and state into it ticket by ticket; a click's pointer input
//! is the first, sent while its caller holds the page's turn, so gestures
//! keep their arrival order (ADR 0085).

use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_engine::tasks::{TaskOwner, TaskStarter};
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument as _;

use crate::cdp::CdpPage;
use crate::errors::Call;

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

impl PageOwner {
    /// Starts the owner of a Page as a child of `session`, its Session's
    /// owner, its loop in `span`, its Run's; once it has gone, each ask
    /// raises `closed`. It ends once its Session's owner is cancelled, or
    /// once every clone of what this answers has gone, after its child
    /// tasks.
    pub(crate) fn start(session: &TaskStarter, span: &tracing::Span, closed: CallError) -> Self {
        let (asks, mailbox) = mpsc::channel(ASKS);
        let owner = session.child();
        session.spawn(serve(owner, mailbox).instrument(span.clone()));
        Self {
            asks,
            closed: Arc::new(closed),
        }
    }

    /// Clicks at (`x`, `y`) in the viewport of `page`, through the owner:
    /// [`CdpPage::pointer_click`] for `call`, as a child task of the owner,
    /// in the caller's span.
    ///
    /// # Errors
    ///
    /// What the click raises; `closed` once the owner has gone.
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
            return Err(CallError::clone(&self.closed));
        }
        answered
            .await
            .unwrap_or_else(|_| Err(CallError::clone(&self.closed)))
    }
}

/// The owner's loop: it takes each ask in turn and starts its work, never
/// awaiting it, until `owner` is cancelled or every [`PageOwner`] has gone, then
/// stops its child tasks.
async fn serve(owner: TaskOwner, mut mailbox: mpsc::Receiver<Ask>) {
    loop {
        let ask = tokio::select! {
            biased;
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
