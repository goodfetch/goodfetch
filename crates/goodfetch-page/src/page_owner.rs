//! A Page's owner (ADR 0146): one task, under its Session's owner, that
//! takes what is asked of its Page and runs the work that waits on the
//! browser as its own child tasks, each answering its caller, so that its
//! loop never awaits the browser or the program. Spec #2256 moves a Page's
//! operations and state into it ticket by ticket. It holds the Page's
//! Gesture order ([`GestureOrder`]) and grants each Gesture its turn, in
//! arrival order (ADR 0085); the Gesture's call then sends its input. It
//! grants a call of the Page API its reads at once ([`Reads`]), while a
//! Gesture holds the turn too, and holds the worlds the engine keeps in the
//! Page's documents ([`PageWorlds`]). Each of the Page's navigations runs
//! as one of its tasks ([`PageOwner::navigation`]), as does the following
//! of its out-of-process frames and dedicated workers
//! (`page_children`), so the Page's close ends them.
//!
//! A panic of one of its tasks closes its Page alone (ADR 0148): the owner
//! keeps `engine_panicked`, naming the Page, which each ask in flight and
//! each later one raises, and which the Session's watch of the Page awaits
//! to close it. A panic of its loop itself is told to its Session's owner.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::tasks::{TaskOwner, TaskStarter};
use goodfetch_protocol::page::PageId;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::Instrument as _;

use crate::cdp::CdpPage;
use crate::consent_walls::Walls;
use crate::dialogs::PageDialogs;
use crate::errors::{page_closed, page_panicked};
use crate::gesture_order::{GestureOrder, GestureTurn};
use crate::hiding::Hiding;
use crate::init_scripts::InitScripts;
use crate::interception::PageInterception;
use crate::page_events::PageEvents;
use crate::page_reads::Reads;
use crate::page_worlds::PageWorlds;

/// How many asks wait for a Page's owner at most: a caller past them waits
/// for room.
const ASKS: usize = 32;

/// What a Page's calls hold of its owner, and ask it through; its clones
/// share one owner.
#[derive(Clone)]
pub(crate) struct PageOwner {
    /// The Page's state the owner holds, which its clones share.
    parts: Arc<PageParts>,
    asks: mpsc::Sender<Ask>,
    /// What an ask raises once the owner has gone: the page's `closed`.
    closed: Arc<CallError>,
    /// `engine_panicked`, once a panic of one of its tasks closed it.
    panicked: watch::Receiver<Option<CallError>>,
    /// Where each ask is counted, its Run's (ADR 0140's
    /// `C2.owner_round_trips` and `C3.owner_round_trips`).
    work: CountedWork,
}

/// The Page's state its owner holds, one part at a time as spec #2256
/// moves it from the engine-wide tables.
pub(crate) struct PageParts {
    /// Its events: the requests and responses it follows, its waiters, its
    /// `page.on` listeners, what a body read needs and the arming of
    /// `Runtime`.
    pub(crate) events: PageEvents,
    /// Its dialog handlers, and the failure one left.
    pub(crate) dialogs: PageDialogs,
    /// Its init scripts, with how many are on their way in or out.
    pub(crate) init_scripts: InitScripts,
    /// Its side of the interception: its Routes, its Filter lists' state,
    /// its handlers' failures and its pending navigation.
    pub(crate) interception: PageInterception,
    /// The consent walls the Consent prompt runner leaves in it, and
    /// whether its first pass on its document goes on.
    pub(crate) walls: Walls,
    /// Its hiding sheets and live hiding's rules in them.
    pub(crate) hiding: Hiding,
    /// The worlds the engine keeps in its documents.
    pub(crate) worlds: PageWorlds,
    /// Its Run's identity that starts tasks on its owner, as its other
    /// parts start theirs, such as a call's reading of robots.txt.
    pub(crate) tasks: RunIdentity,
}

/// The open Pages' owners, by their ID: the index from ID to handle (ADR
/// 0146), written when a Page is kept and when it ends, which a call that
/// acts as it arrives, before anything is awaited, reads. Clones share it.
#[derive(Clone, Default)]
pub(crate) struct PageOwners(
    #[expect(
        clippy::disallowed_types,
        reason = "one map read and changed for one operation, never across another lock or an `.await`; the Sessions table's lock may be held around it, and is never taken under it, so there is no lock order to test under loom (ADR 0060)"
    )]
    Arc<std::sync::Mutex<BTreeMap<PageId, PageOwner>>>,
);

/// One thing asked of a Page's owner, with where its answer goes.
enum Ask {
    /// A Gesture's turn.
    Turn(oneshot::Sender<GestureTurn>),
    /// A call's reads, granted at once.
    Read(oneshot::Sender<()>),
    /// Work that waits on the browser, such as a navigation, started as one
    /// of the owner's tasks, which answers its caller itself.
    Start(Pin<Box<dyn Future<Output = ()> + Send>>),
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
    /// Starts the owner of the Page `page_id`, holding `parts` and the
    /// Gesture order `order`, its loop a task of `session`, its Session's
    /// owner, and its own tasks those of `tasks`, a child of `session` that
    /// the Page's opening may have started tasks on already; its loop runs
    /// in `span`, its Run's, counting on `metrics`, its Run's. Once it has
    /// gone, each ask raises `closed`, or `engine_panicked` after a panic of
    /// one of its tasks. It ends once its Session's owner is cancelled,
    /// once one of its tasks panicked, or once every clone of what this
    /// answers has gone, after its tasks.
    pub(crate) fn start(
        (session, owner): (&TaskStarter, TaskOwner),
        span: &tracing::Span,
        (page_id, parts, order): (&PageId, PageParts, GestureOrder),
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
        session.spawn(serve(owner, mailbox, order, panic).instrument(span.clone()));
        Self {
            parts: Arc::new(parts),
            asks,
            closed: Arc::new(closed),
            panicked,
            work: metrics.counted_work().clone(),
        }
    }

    /// Its Page's events.
    pub(crate) fn events(&self) -> &PageEvents {
        &self.parts.events
    }

    /// Its Page's dialog handlers.
    pub(crate) fn dialogs(&self) -> &PageDialogs {
        &self.parts.dialogs
    }

    /// Its Page's init scripts.
    pub(crate) fn init_scripts(&self) -> &InitScripts {
        &self.parts.init_scripts
    }

    /// Its Page's side of the interception.
    pub(crate) fn interception(&self) -> &PageInterception {
        &self.parts.interception
    }

    /// Its Page's consent walls.
    pub(crate) fn consent_walls(&self) -> &Walls {
        &self.parts.walls
    }

    /// Its Page's hiding.
    pub(crate) fn hiding(&self) -> &Hiding {
        &self.parts.hiding
    }

    /// The worlds the engine keeps in its Page's documents.
    pub(crate) fn worlds(&self) -> &PageWorlds {
        &self.parts.worlds
    }

    /// Its Run's identity that starts tasks on it, which its end waits for.
    pub(crate) fn tasks(&self) -> &RunIdentity {
        &self.parts.tasks
    }

    /// The reads of a call of the Page API on the page `cdp` drives,
    /// granted at once, while a Gesture holds the turn too. One round trip
    /// to the owner, counted on `C2.owner_round_trips`.
    ///
    /// # Errors
    ///
    /// Once the owner has gone, `closed`, or `engine_panicked` when a panic
    /// of one of its tasks closed it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the ask.
    pub(crate) async fn reads(&self, cdp: CdpPage) -> Result<Reads, CallError> {
        self.work.count(Row::PageReadOwnerRoundTrips);
        let (answer, answered) = oneshot::channel();
        if self.asks.send(Ask::Read(answer)).await.is_err() {
            return Err(self.gone().await);
        }
        match answered.await {
            Ok(()) => Ok(Reads::of(cdp)),
            Err(_) => Err(self.gone().await),
        }
    }

    /// Runs `work`, one of the Page's navigations, as a task of the owner,
    /// beside its other asks, and answers what it answers. It counts no
    /// round trip: a navigation waits on the browser far longer.
    ///
    /// # Errors
    ///
    /// What `work` raises; once the owner has gone, as when the Page
    /// closes during it, `closed`, or `engine_panicked` when a panic of one
    /// of its tasks closed it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future ends `work` at its next wait.
    pub(crate) async fn navigation<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, CallError>> + Send + 'static,
    ) -> Result<T, CallError> {
        let (mut answer, answered) = oneshot::channel();
        let started = Box::pin(async move {
            let outcome = tokio::select! {
                biased;
                () = answer.closed() => return,
                outcome = work => outcome,
            };
            // Its caller may have given up the wait.
            let _ = answer.send(outcome);
        });
        if self.asks.send(Ask::Start(started)).await.is_err() {
            return Err(self.gone().await);
        }
        match answered.await {
            Ok(outcome) => outcome,
            Err(_) => Err(self.gone().await),
        }
    }

    /// Waits for a Gesture's turn on the Page, in arrival order. One round
    /// trip to the owner, counted on `C3.owner_round_trips`.
    ///
    /// # Errors
    ///
    /// Once the owner has gone, `closed`, or `engine_panicked` when a panic
    /// of one of its tasks closed it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the place in the order.
    pub(crate) async fn turn(&self) -> Result<GestureTurn, CallError> {
        self.work.count(Row::ActionOwnerRoundTrips);
        let (answer, answered) = oneshot::channel();
        if self.asks.send(Ask::Turn(answer)).await.is_err() {
            return Err(self.gone().await);
        }
        match answered.await {
            Ok(turn) => Ok(turn),
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

impl PageOwners {
    /// Locks the index, whose holders never panic while they hold it.
    fn held(&self) -> std::sync::MutexGuard<'_, BTreeMap<PageId, PageOwner>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Lists `owner`, the owner of the Page `page_id`, kept now.
    pub(crate) fn kept(&self, page_id: PageId, owner: PageOwner) {
        self.held().insert(page_id, owner);
    }

    /// The owner of the open Page `page_id`.
    pub(crate) fn get(&self, page_id: &PageId) -> Option<PageOwner> {
        self.held().get(page_id).cloned()
    }

    /// The events of the open Page `page_id`, or `closed`.
    pub(crate) fn events(&self, page_id: &PageId) -> Result<PageEvents, CallError> {
        self.held()
            .get(page_id)
            .map(|owner| owner.events().clone())
            .ok_or_else(|| page_closed(page_id))
    }

    /// The dialog handlers of the open Page `page_id`, or `closed`.
    pub(crate) fn dialogs(&self, page_id: &PageId) -> Result<PageDialogs, CallError> {
        self.held()
            .get(page_id)
            .map(|owner| owner.dialogs().clone())
            .ok_or_else(|| page_closed(page_id))
    }

    /// The wait until no init script of the open Page `page_id` is on its
    /// way in or out; none while none is, or for a Page that is not open.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) fn settling(
        &self,
        page_id: &PageId,
    ) -> Option<impl Future<Output = ()> + Send + use<>> {
        self.held().get(page_id)?.init_scripts().settling()
    }

    /// The wait until no `page.on` of the open Page `page_id` sent before
    /// now is still turning `Runtime` on; none while none is, or for a Page
    /// that is not open.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) fn arming(
        &self,
        page_id: &PageId,
    ) -> Option<impl Future<Output = ()> + Send + use<>> {
        self.held().get(page_id)?.events().arming()
    }

    /// Takes the Pages `pages` out, as each ends: each one's events close,
    /// their calls answering, then its dialogs, then its side of the
    /// interception.
    pub(crate) fn close(&self, pages: &[PageId]) {
        let ended: Vec<PageOwner> = {
            let mut held = self.held();
            pages.iter().filter_map(|page| held.remove(page)).collect()
        };
        for owner in &ended {
            owner.events().close();
            owner.dialogs().close();
            owner.interception().close();
        }
    }
}

/// The owner's loop: it takes in each ask, and each change of the Gesture
/// order `order`, as it comes, never awaiting the browser, until `owner` is
/// cancelled, one of its tasks panicked, which `panic` keeps, or every
/// [`PageOwner`] has gone, then stops its child tasks.
async fn serve(
    owner: TaskOwner,
    mut mailbox: mpsc::Receiver<Ask>,
    mut order: GestureOrder,
    panic: Panicked,
) {
    let panicked = owner.panicked();
    tokio::pin!(panicked);
    loop {
        tokio::select! {
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
            () = order.changed() => {}
            ask = mailbox.recv() => match ask {
                Some(Ask::Turn(answer)) => order.ask(answer),
                Some(Ask::Read(answer)) => {
                    // Its caller may have given up the ask.
                    let _ = answer.send(());
                }
                Some(Ask::Start(work)) => {
                    let token = owner.child_token();
                    owner.spawn(async move {
                        tokio::select! {
                            biased;
                            () = token.cancelled() => {}
                            () = work => {}
                        }
                    });
                }
                None => break,
            },
        }
    }
    // What waits for its turn has gone with the page.
    drop(order);
    owner.stop().await;
}

#[cfg(test)]
pub(crate) mod tests;
