//! The Connections that serve a crawl's handler requests (ADRs 0024 and
//! 0081): the one that started the crawl, then each that calls
//! `crawl.serve`, as `workers=N` handler processes do through an Attached
//! engine's private endpoint or a Daemon. A crawl's delivery takes each
//! Connection that joins its live Run, and leases work items to it until
//! it leaves, when its `crawl.serve` call ends, or its Connection ends. A
//! handler's enqueue and a seed added to the running crawl reach the live
//! crawl's Run through it too, and a seed added wakes its delivery, while
//! its intake is open: a crawl about to end closes it ([`Intake`]). Each
//! live crawl's attempts stage their outputs in its staging, which its Run
//! also takes the Key-value store writes made through an attempt in, and
//! check each Record they push or commit against its Datasets, read and
//! built once as it opened (ADR 0138).

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::Datasets;
use goodfetch_engine::CallContext;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::foxhound::crawl::CRAWL_MAX_STAGED_ENQUEUES;
use goodfetch_protocol::foxhound::settings::DATASET_RECORDS;
use goodfetch_protocol::run::RunId;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use crate::intake::Intake;
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
    intake: Intake,
    datasets: Datasets,
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
    /// Whether it still takes new work, which a write giving it work
    /// checks in its own job.
    pub(crate) intake: Intake,
    /// The Datasets it declares, which each pushed Record is checked
    /// against.
    pub(crate) datasets: Datasets,
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
    /// Whether it still takes new work, which its delivery closes before
    /// it ends the Run.
    pub(crate) intake: Intake,
    /// The Datasets it declares, which each committed Record is checked
    /// against.
    pub(crate) datasets: Datasets,
}

/// How many joining Connections wait for a crawl's delivery to take them:
/// one more waits in its `crawl.serve` call until the delivery takes one.
const JOINING: usize = 16;

/// The live crawls of one engine.
#[derive(Clone, Default)]
pub(crate) struct Crawls(Arc<Mutex<BTreeMap<RunId, Live>>>);

/// Whether the crawl `run` keeps Records, its `dataset.records` (ADR 0138):
/// off, it checks and commits none.
pub(crate) fn keeps_records(run: &Run) -> bool {
    run.settings()
        .and_then(|settings| settings.resolved().boolean(DATASET_RECORDS.name))
        != Some(false)
}

impl Crawls {
    /// Opens the crawl `run`, under its domain policy `egress` and with the
    /// Datasets it declares, keeping Records in them as its
    /// `dataset.records` says, to Connections that join it and to seeds
    /// added to it, and answers where each arrives.
    pub(crate) async fn open(&self, run: &Run, egress: Policy, datasets: Datasets) -> Opened {
        let datasets = datasets.keeping_records(keeps_records(run));
        let (joining, joined) = mpsc::channel(JOINING);
        let seeded = Arc::new(Notify::new());
        let staging = run
            .settings()
            .and_then(|settings| settings.resolved().count(CRAWL_MAX_STAGED_ENQUEUES.name))
            .map_or_else(Staging::default, Staging::bounded);
        run.stage_attempt_writes_through(Arc::new(staging.clone()));
        let egress = Arc::new(egress);
        let intake = Intake::default();
        self.0.lock().await.insert(
            run.id().clone(),
            Live {
                run: run.clone(),
                egress: Arc::clone(&egress),
                joining,
                seeded: Arc::clone(&seeded),
                staging: staging.clone(),
                intake: intake.clone(),
                datasets: datasets.clone(),
            },
        );
        Opened {
            joined,
            seeded,
            staging,
            egress,
            intake,
            datasets,
        }
    }

    /// What adds to the live crawl `run_id`, if this engine runs it.
    pub(crate) async fn live(&self, run_id: &RunId) -> Option<LiveCrawl> {
        self.0.lock().await.get(run_id).map(|live| LiveCrawl {
            run: live.run.clone(),
            egress: Arc::clone(&live.egress),
            seeded: Arc::clone(&live.seeded),
            staging: live.staging.clone(),
            intake: live.intake.clone(),
            datasets: live.datasets.clone(),
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
