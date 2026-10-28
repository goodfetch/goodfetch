//! The Connections that serve a crawl's handler requests (ADRs 0024 and
//! 0081): the one that started the crawl, then each that calls
//! `crawl.serve`, as `workers=N` handler processes do through an Attached
//! engine's private endpoint or a Daemon. A crawl's delivery takes each
//! Connection that joins its live Run, and leases work items to it until
//! it leaves, when its `crawl.serve` call ends, or its Connection ends. A
//! handler's enqueue and a seed added to the running crawl reach the live
//! crawl's Run through it too, and a seed added wakes its delivery. Each
//! live crawl's attempts stage their outputs in its staging, which its Run
//! also takes the Key-value store writes made through an attempt in.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::CallContext;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::run::RunId;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use crate::staging::Staging;

/// A Connection that joined a crawl, and the sign that it left: the
/// sending half of `left` drops when its `crawl.serve` call ends.
pub(crate) struct Server {
    /// The context of its call, whose Connection's Client its handler
    /// requests go to and its handlers' browser pages are opened for.
    pub(crate) context: CallContext,
    /// Closed once it no longer takes new work items.
    pub(crate) left: oneshot::Receiver<()>,
}

/// A live crawl: its Run, the way in for Connections that join it, and
/// the sign that a seed was added to it.
struct Live {
    run: Run,
    egress: Arc<Policy>,
    joining: mpsc::Sender<Server>,
    seeded: Arc<Notify>,
    staging: Staging,
}

/// What a seed added to a live crawl, or a handler's enqueue, reaches:
/// its Run, the domain policy its Frontier judges each URL by, and what
/// tells its delivery that seeds were added.
pub(crate) struct LiveCrawl {
    /// The crawl's Run.
    pub(crate) run: Run,
    /// The Run's domain policy.
    pub(crate) egress: Arc<Policy>,
    /// Notified once seeds were added.
    pub(crate) seeded: Arc<Notify>,
    /// Where its attempts stage their outputs.
    pub(crate) staging: Staging,
}

/// A crawl opened to Connections that join it and to seeds added to it.
pub(crate) struct Opened {
    /// Where joining Connections arrive.
    pub(crate) joined: mpsc::Receiver<Server>,
    /// Notified once seeds were added since its last notification.
    pub(crate) seeded: Arc<Notify>,
    /// Where its attempts stage their outputs until they commit.
    pub(crate) staging: Staging,
    /// The Run's domain policy, which judges each redirect hop.
    pub(crate) egress: Arc<Policy>,
}

/// How many joining Connections wait for a crawl's delivery to take them:
/// one more waits in its `crawl.serve` call until the delivery takes one.
const JOINING: usize = 16;

/// The live crawls of one engine.
#[derive(Clone, Default)]
pub(crate) struct Crawls(Arc<Mutex<BTreeMap<RunId, Live>>>);

impl Crawls {
    /// Opens the crawl `run`, under its domain policy `egress`, to
    /// Connections that join it and to seeds added to it, and answers where
    /// each arrives.
    pub(crate) async fn open(&self, run: &Run, egress: Policy) -> Opened {
        let (joining, joined) = mpsc::channel(JOINING);
        let seeded = Arc::new(Notify::new());
        let staging = Staging::default();
        run.stage_attempt_writes_through(Arc::new(staging.clone()));
        let egress = Arc::new(egress);
        self.0.lock().await.insert(
            run.id().clone(),
            Live {
                run: run.clone(),
                egress: Arc::clone(&egress),
                joining,
                seeded: Arc::clone(&seeded),
                staging: staging.clone(),
            },
        );
        Opened {
            joined,
            seeded,
            staging,
            egress,
        }
    }

    /// What adds to the live crawl `run_id`, if this engine runs it.
    pub(crate) async fn live(&self, run_id: &RunId) -> Option<LiveCrawl> {
        self.0.lock().await.get(run_id).map(|live| LiveCrawl {
            run: live.run.clone(),
            egress: Arc::clone(&live.egress),
            seeded: Arc::clone(&live.seeded),
            staging: live.staging.clone(),
        })
    }

    /// Closes the crawl `run_id`: no Connection joins it again.
    pub(crate) async fn close(&self, run_id: &RunId) {
        self.0.lock().await.remove(run_id);
    }

    /// Joins the Connection of the call `context` to the live crawl
    /// `run_id`, until the answered sender drops; `None` when no such crawl
    /// is open, or it ended before its delivery took the Connection. While
    /// [`JOINING`] Connections already wait for its delivery, it waits too.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it answers joins nothing.
    pub(crate) async fn join(
        &self,
        run_id: &RunId,
        context: CallContext,
    ) -> Option<oneshot::Sender<()>> {
        let (stays, left) = oneshot::channel();
        let joining = self.0.lock().await.get(run_id)?.joining.clone();
        joining.send(Server { context, left }).await.ok()?;
        Some(stays)
    }
}
