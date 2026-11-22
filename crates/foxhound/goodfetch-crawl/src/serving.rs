//! The Connections that serve a crawl's handler requests (ADRs 0024 and
//! 0081): the one that started the crawl, then each that calls
//! `crawl.serve`, as `workers=N` handler processes do through an Attached
//! engine's private endpoint or a Daemon. A crawl's delivery takes each
//! Connection that joins its live Run, and leases work items to it until
//! it leaves, when its `crawl.serve` call ends, or its Connection ends. A
//! handler's enqueue, a seed added to the running crawl and a re-admission
//! reach the live crawl's Run state through it too, over the mailbox of
//! the delivery loop that owns it (ADR 0142), while its intake is open: a
//! crawl about to end closes it ([`Intake`]). Each
//! live crawl's attempts stage their outputs in its staging, which the
//! engine also stages the Key-value store writes made through an attempt
//! in, asking these live crawls by Run ID ([`AttemptStaging`]), and
//! check each Record they push or commit against its Datasets, read and
//! built once as it opened (ADR 0138).

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::Datasets;
use goodfetch_engine::CallContext;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_engine::staging::{AttemptStaging, AttemptWrites, StagingOf};
use goodfetch_protocol::foxhound::crawl::CRAWL_MAX_STAGED_ENQUEUES;
use goodfetch_protocol::foxhound::settings::DATASET_RECORDS;
use goodfetch_protocol::run::RunId;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::crawl_run::CrawlRun;
use crate::intake::Intake;
use crate::invariants::Invariants;
use crate::run_state::{self, Asked, Asking};
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
/// how other tasks ask its Run state.
struct Live {
    run: Arc<CrawlRun>,
    egress: Arc<Policy>,
    invariants: Invariants,
    joining: mpsc::Sender<Server>,
    asking: Asking,
    staging: Staging,
    datasets: Datasets,
}

/// What a seed added to a live crawl, or a handler's enqueue, reaches:
/// its Run, the domain policy its Frontier judges each URL by, and how it
/// asks the crawl's Run state.
pub(crate) struct LiveCrawl {
    /// The crawl's record of its Run.
    pub(crate) run: Arc<CrawlRun>,
    /// The Run's domain policy.
    pub(crate) egress: Arc<Policy>,
    /// What each of its admissions reads, built once.
    pub(crate) invariants: Invariants,
    /// How a call asks the crawl's Run state, which its delivery loop
    /// owns.
    pub(crate) asking: Asking,
    /// Where its attempts stage their outputs.
    pub(crate) staging: Staging,
    /// The Datasets it declares, which each pushed Record is checked
    /// against.
    pub(crate) datasets: Datasets,
}

/// A crawl opened to Connections that join it and to seeds added to it.
pub(crate) struct Opened {
    /// Where joining Connections arrive.
    pub(crate) joined: mpsc::Receiver<Server>,
    /// What other tasks ask of its Run state, for its delivery loop to
    /// answer.
    pub(crate) asked: Asked,
    /// How its pages' redirect chains ask the same.
    pub(crate) asking: Asking,
    /// Where its attempts stage their outputs until they commit.
    pub(crate) staging: Staging,
    /// The Run's domain policy, which judges each redirect hop.
    pub(crate) egress: Arc<Policy>,
    /// What each of its admissions reads, built once.
    pub(crate) invariants: Invariants,
    /// Whether it still takes new work, which each job that gives it
    /// work checks, and its delivery closes before it ends the Run.
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
pub(crate) fn keeps_records(run: &RunIdentity) -> bool {
    get_or_default(run.settings().map(RunSettings::resolved), &DATASET_RECORDS)
}

impl Crawls {
    /// Opens the crawl `run`, under its domain policy `egress`, with the
    /// `invariants` its admissions read and the Datasets it declares,
    /// keeping Records in them as its `dataset.records` says, to
    /// Connections that join it and to seeds added to it, and answers where
    /// each arrives.
    pub(crate) async fn open(
        &self,
        run: &Arc<CrawlRun>,
        egress: Policy,
        invariants: Invariants,
        datasets: Datasets,
    ) -> Opened {
        let identity = &run.identity;
        let datasets = datasets.keeping_records(keeps_records(identity));
        let (joining, joined) = mpsc::channel(JOINING);
        let (asking, asked) = run_state::mailbox(identity.metrics().counted_work().clone());
        let staging = Staging::bounded(get_or_default(
            identity.settings().map(RunSettings::resolved),
            &CRAWL_MAX_STAGED_ENQUEUES,
        ));
        let egress = Arc::new(egress);
        let intake = Intake::default();
        self.0.lock().await.insert(
            identity.id().clone(),
            Live {
                run: Arc::clone(run),
                egress: Arc::clone(&egress),
                invariants: invariants.clone(),
                joining,
                asking: asking.clone(),
                staging: staging.clone(),
                datasets: datasets.clone(),
            },
        );
        Opened {
            joined,
            asked,
            asking,
            staging,
            egress,
            invariants,
            intake,
            datasets,
        }
    }

    /// What adds to the live crawl `run_id`, if this engine runs it.
    pub(crate) async fn live(&self, run_id: &RunId) -> Option<LiveCrawl> {
        self.0.lock().await.get(run_id).map(|live| LiveCrawl {
            run: Arc::clone(&live.run),
            egress: Arc::clone(&live.egress),
            invariants: live.invariants.clone(),
            asking: live.asking.clone(),
            staging: live.staging.clone(),
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

impl AttemptStaging for Crawls {
    /// The staging of the live crawl `run_id`, while its Run's end is not
    /// decided: a crawl is closed only after its delivery ended its Run,
    /// or never when its delivery is cancelled, and a write staged once
    /// `Runs::live` no longer finds the Run answers `run_not_found`.
    fn staging_of<'a>(&'a self, run_id: &'a RunId) -> StagingOf<'a> {
        Box::pin(async move {
            let crawls = self.0.lock().await;
            let live = crawls
                .get(run_id)
                .filter(|live| !live.run.end.is_decided())?;
            let writes: Arc<dyn AttemptWrites> = Arc::new(live.staging.clone());
            Some(writes)
        })
    }
}
