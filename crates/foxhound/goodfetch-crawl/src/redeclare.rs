//! The Datasets a start under the ID of a Run declares when no crawl of
//! this engine works that Run (ADRs 0102 and 0103): the start checked them
//! against those the Run began with, and they are kept until the Run
//! resumes here, when they are recorded in its run file before any work,
//! so the Records committed from then on are checked against a changed
//! schema and keep its hash. A start under the ID of a crawl this engine
//! works changes none of its declarations, as it changes none of its
//! Labels.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::Datasets;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::run::{RunId, RunStarted};
use tokio::sync::Mutex;

use crate::serving::Crawls;

/// The declarations kept for each Run until it resumes.
#[derive(Clone, Default)]
pub(crate) struct Redeclared(Arc<Mutex<BTreeMap<RunId, Datasets>>>);

impl Redeclared {
    /// Keeps `datasets` for the resume of the Run a start under its ID
    /// found, in place of any kept before, unless a crawl of `crawls` works
    /// it; answers how that start found it.
    pub(crate) async fn keep(
        &self,
        found: RunStarted,
        datasets: Datasets,
        crawls: &Crawls,
    ) -> RunStarted {
        if crawls.live(&found.run_id).await.is_none() {
            self.0.lock().await.insert(found.run_id.clone(), datasets);
        }
        found
    }

    /// Records the declarations kept for the resuming `run`, if any, in its
    /// run file. A write that fails is logged: the Run goes on under the
    /// declarations it held.
    pub(crate) async fn record(&self, run: &Run) {
        let Some(datasets) = self.0.lock().await.remove(run.id()) else {
            return;
        };
        if let Err(error) = run
            .state()
            .write(move |connection| datasets.record_again(connection))
            .await
        {
            tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's Datasets were not recorded again");
        }
    }
}
