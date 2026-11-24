//! A Page's Gesture order (ADRs 0085 and 0146), which the Page's owner
//! holds as a value of its loop: who waits for the turn, in arrival order,
//! and where the page's pointer is. The pointer goes with the turn: a
//! Gesture holds both from its first command to its last, and lends the
//! pointer between two of its checks to what else clicks on the page, the
//! Consent prompt runner, so the runner clears the prompt that covers the
//! Gesture's target. With no turn held, the owner lends the pointer itself.
//! A turn and a loan end as they are dropped, however their holder ends, so
//! neither is a lock, and the owner's loop never waits on the browser.

use std::collections::VecDeque;

use tokio::sync::{mpsc, oneshot};

/// How many asks for a page's pointer wait at most: one past them waits for
/// room.
const ASKS: usize = 32;

/// Where a page's pointer is asked for; its clones ask one order.
#[derive(Clone)]
pub(crate) struct PointerAsks(mpsc::Sender<oneshot::Sender<LentPointer>>);

/// A page's pointer, lent until dropped.
pub(crate) struct LentPointer {
    /// Dropped with the loan, which the lender awaits.
    _back: oneshot::Sender<()>,
}

/// A page's pointer, at whom it lends to what asks for it: the Gesture
/// whose turn it is, or the order.
struct Pointer {
    asks: mpsc::Receiver<oneshot::Sender<LentPointer>>,
    /// Closed once the loan under way ended; none while none is.
    lent: Option<oneshot::Receiver<()>>,
}

/// A Page's Gesture order.
pub(crate) struct GestureOrder {
    at: At,
    /// The Gestures that wait for the turn, the oldest first.
    waiting: VecDeque<oneshot::Sender<GestureTurn>>,
}

/// Where the turn is.
enum At {
    /// No Gesture's: the order holds the pointer.
    Free(Pointer),
    /// A Gesture's, which gives the pointer back as its turn ends.
    Held(oneshot::Receiver<Pointer>),
}

/// A Gesture's turn on its page, until dropped: the pointer is its own.
pub(crate) struct GestureTurn {
    /// Its pointer, and where it goes back to; taken as it drops.
    pointer: Option<(Pointer, oneshot::Sender<Pointer>)>,
}

impl PointerAsks {
    /// Waits for the page's pointer, in arrival order; none once the page's
    /// owner has gone.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the place in the order.
    pub(crate) async fn lent(&self) -> Option<LentPointer> {
        let (answer, answered) = oneshot::channel();
        self.0.send(answer).await.ok()?;
        answered.await.ok()
    }
}

impl Pointer {
    /// Lends it to the caller of `ask`, if that still waits.
    fn lend(&mut self, ask: oneshot::Sender<LentPointer>) {
        let (back, lent) = oneshot::channel();
        if ask.send(LentPointer { _back: back }).is_ok() {
            self.lent = Some(lent);
        }
    }

    /// Waits until no loan is under way.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the loan as it is.
    async fn returned(&mut self) {
        if let Some(lent) = &mut self.lent {
            // Nothing is sent: the loan's drop closes it.
            let _ = lent.await;
            self.lent = None;
        }
    }
}

impl GestureOrder {
    /// The order of a page no Gesture acts on yet, and where its pointer is
    /// asked for, which what clicks on the page holds before its owner
    /// starts.
    pub(crate) fn new() -> (Self, PointerAsks) {
        let (asks, asked) = mpsc::channel(ASKS);
        let order = Self {
            at: At::Free(Pointer {
                asks: asked,
                lent: None,
            }),
            waiting: VecDeque::new(),
        };
        (order, PointerAsks(asks))
    }

    /// Takes the Gesture `answer` answers to into the order, whose turn it
    /// gets there once it is the oldest and nothing else holds the pointer.
    pub(crate) fn ask(&mut self, answer: oneshot::Sender<GestureTurn>) {
        // What gave up its place is no longer waited for.
        self.waiting.retain(|waiting| !waiting.is_closed());
        self.waiting.push_back(answer);
        self.grant();
    }

    /// Waits for the order to change other than by [`Self::ask`], and takes
    /// the change in: a turn or a loan ends, or the pointer is asked for
    /// while no Gesture has it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no change: the next call takes it in.
    pub(crate) async fn changed(&mut self) {
        match &mut self.at {
            At::Held(held) => {
                let pointer = held
                    .await
                    .expect("invariant: a turn gives its pointer back as it drops");
                self.at = At::Free(pointer);
            }
            At::Free(pointer) if pointer.lent.is_some() => pointer.returned().await,
            At::Free(pointer) => match pointer.asks.recv().await {
                Some(ask) => pointer.lend(ask),
                // Nothing asks for the pointer any more.
                None => std::future::pending().await,
            },
        }
        self.grant();
    }

    /// Grants the oldest Gesture that waits its turn, once nothing holds
    /// the pointer. One whose caller has gone drops it, which gives the
    /// turn back.
    fn grant(&mut self) {
        let At::Free(Pointer { lent: None, .. }) = &self.at else {
            return;
        };
        let Some(answer) = self.waiting.pop_front() else {
            return;
        };
        let (back, held) = oneshot::channel();
        if let At::Free(pointer) = std::mem::replace(&mut self.at, At::Held(held)) {
            let _ = answer.send(GestureTurn {
                pointer: Some((pointer, back)),
            });
        }
    }
}

impl GestureTurn {
    /// Lends the pointer to the oldest of what asks for it now, between two
    /// of the Gesture's checks, and waits until it is back.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the loan as it is: the next Gesture, or
    /// this one's next lending, waits for its end.
    pub(crate) async fn lend_pointer(&mut self) {
        let Some((pointer, _)) = &mut self.pointer else {
            return;
        };
        pointer.returned().await;
        while pointer.lent.is_none()
            && let Ok(ask) = pointer.asks.try_recv()
        {
            pointer.lend(ask);
        }
        pointer.returned().await;
    }
}

impl Drop for GestureTurn {
    fn drop(&mut self) {
        if let Some((pointer, back)) = self.pointer.take() {
            // The page's owner may have gone: nothing waits for it then.
            let _ = back.send(pointer);
        }
    }
}
