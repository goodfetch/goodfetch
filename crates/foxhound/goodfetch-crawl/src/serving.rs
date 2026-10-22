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

use goodfetch_engine::client::Client;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::run::RunId;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

/// A Connection that joined a crawl, and the sign that it left: the
/// sending half of `left` drops when its `crawl.serve` call ends.
pub(crate) struct Server {
    /// The Client its handler requests go to.
    pub(crate) client: Client,
    /// Closed once it no longer takes new work items.
    pub(crate) left: oneshot::Receiver<()>,
}

/// A live crawl: its Run, the way in for Connections that join it, and
/// the sign that a seed was added to it.
struct Live {
    run: Run,
    joining: mpsc::UnboundedSender<Server>,
    seeded: Arc<Notify>,
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
    /// Opens the crawl `run` to Connections that join it and to seeds
    /// added to it, and answers where each arrives.
    pub(crate) async fn open(&self, run: &Run) -> Opened {
        let (joining, joined) = mpsc::unbounded_channel();
        let seeded = Arc::new(Notify::new());
        self.0.lock().await.insert(
            run.id().clone(),
            Live {
                run: run.clone(),
                joining,
                seeded: Arc::clone(&seeded),
            },
        );
        Opened { joined, seeded }
    }

    /// The live crawl `run_id`'s Run, if this engine runs it.
    pub(crate) async fn run(&self, run_id: &RunId) -> Option<Run> {
        self.0.lock().await.get(run_id).map(|live| live.run.clone())
    }

    /// The live crawl `run_id`'s Run, if this engine runs it, and what
    /// tells its delivery that seeds were added.
    pub(crate) async fn seeding(&self, run_id: &RunId) -> Option<(Run, Arc<Notify>)> {
        self.0
            .lock()
            .await
            .get(run_id)
            .map(|live| (live.run.clone(), Arc::clone(&live.seeded)))
    }

    /// Closes the crawl `run_id`: no Connection joins it again.
    pub(crate) async fn close(&self, run_id: &RunId) {
        self.0.lock().await.remove(run_id);
    }

    /// Joins `client` to the live crawl `run_id`, until the answered
    /// sender drops; `None` when no such crawl is open.
    pub(crate) async fn join(&self, run_id: &RunId, client: Client) -> Option<oneshot::Sender<()>> {
        let (stays, left) = oneshot::channel();
        let crawls = self.0.lock().await;
        let live = crawls.get(run_id)?;
        live.joining.send(Server { client, left }).ok()?;
        Some(stays)
    }
}
