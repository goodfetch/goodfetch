//! The Connections that serve a crawl's handler requests (ADRs 0024 and
//! 0081): the one that started the crawl, then each that calls
//! `crawl.serve`, as `workers=N` handler processes do through an Attached
//! engine's private endpoint or a Daemon. A crawl's delivery takes each
//! Connection that joins its live Run, and leases work items to it until
//! it leaves, when its `crawl.serve` call ends, or its Connection ends. A
//! handler's enqueue and a seed added to the running crawl reach the live
//! crawl's Run through it too, and a seed added wakes its delivery.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::CallContext;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::run::RunId;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

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
    joining: mpsc::UnboundedSender<Server>,
    seeded: Arc<Notify>,
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
}

/// A crawl opened to Connections that join it and to seeds added to it.
pub(crate) struct Opened {
    /// Where joining Connections arrive.
    pub(crate) joined: mpsc::UnboundedReceiver<Server>,
    /// Notified once seeds were added since its last notification.
    pub(crate) seeded: Arc<Notify>,
}

/// The live crawls of one engine.
#[derive(Clone, Default)]
pub(crate) struct Crawls(Arc<Mutex<BTreeMap<RunId, Live>>>);

impl Crawls {
    /// Opens the crawl `run`, under its domain policy `egress`, to
    /// Connections that join it and to seeds added to it, and answers where
    /// each arrives.
    pub(crate) async fn open(&self, run: &Run, egress: Policy) -> Opened {
        let (joining, joined) = mpsc::unbounded_channel();
        let seeded = Arc::new(Notify::new());
        self.0.lock().await.insert(
            run.id().clone(),
            Live {
                run: run.clone(),
                egress: Arc::new(egress),
                joining,
                seeded: Arc::clone(&seeded),
            },
        );
        Opened { joined, seeded }
    }

    /// What adds to the live crawl `run_id`, if this engine runs it.
    pub(crate) async fn live(&self, run_id: &RunId) -> Option<LiveCrawl> {
        self.0.lock().await.get(run_id).map(|live| LiveCrawl {
            run: live.run.clone(),
            egress: Arc::clone(&live.egress),
            seeded: Arc::clone(&live.seeded),
        })
    }

    /// Closes the crawl `run_id`: no Connection joins it again.
    pub(crate) async fn close(&self, run_id: &RunId) {
        self.0.lock().await.remove(run_id);
    }

    /// Joins the Connection of the call `context` to the live crawl
    /// `run_id`, until the answered sender drops; `None` when no such crawl
    /// is open.
    pub(crate) async fn join(
        &self,
        run_id: &RunId,
        context: CallContext,
    ) -> Option<oneshot::Sender<()>> {
        let (stays, left) = oneshot::channel();
        let crawls = self.0.lock().await;
        let live = crawls.get(run_id)?;
        live.joining.send(Server { context, left }).ok()?;
        Some(stays)
    }
}
