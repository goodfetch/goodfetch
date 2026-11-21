//! The Datasets, the Labels and the code-free extraction a start under the
//! ID of a Run declares when no crawl of this engine works that Run (ADRs
//! 0102 and 0103): the start checked the Datasets against those the Run
//! began with, and the Labels against its pending requests, and all are
//! kept until the Run resumes here, when they are recorded in its run file
//! in one write before any work, so the Records committed from then on are
//! read by the new extraction, checked against the schema generated from
//! it, and keep its hash, and each enqueue is checked against the new
//! Labels, a new one accepted. Its Hooks in code are kept beside them and
//! handed to the resumed delivery, never recorded (ADR 0104). A start under
//! the ID of a crawl this engine works changes none of its declarations.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::Datasets;
use goodfetch_engine::runs::Run;
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_protocol::foxhound::crawl::CrawlExtraction;
use goodfetch_protocol::foxhound::hook::CrawlHookDeclaration;
use goodfetch_protocol::run::RunId;
use tokio::sync::Mutex;

use crate::code_free;
use crate::serving::Crawls;

/// What a start under the ID of a Run declared: its Datasets, its Labels,
/// as the rules its Payloads are checked by, the extraction of a
/// code-free crawl, none for one its program handles, and its Hooks in
/// code.
struct Declared {
    datasets: Datasets,
    labels: PayloadRules,
    extraction: Option<CrawlExtraction>,
    hooks: Vec<CrawlHookDeclaration>,
}

/// The declarations kept for each Run until it resumes.
#[derive(Clone, Default)]
pub(crate) struct Redeclared(Arc<Mutex<BTreeMap<RunId, Declared>>>);

impl Redeclared {
    /// Keeps `datasets`, the rules of the Labels `labels`, `extraction`
    /// and `hooks` for the resume of the Run `run_id` a start under its ID found, in
    /// place of any kept before, unless a crawl of `crawls` works it.
    pub(crate) async fn keep(
        &self,
        run_id: &RunId,
        (datasets, labels, extraction): (Datasets, PayloadRules, Option<CrawlExtraction>),
        hooks: Vec<CrawlHookDeclaration>,
        crawls: &Crawls,
    ) {
        if crawls.live(run_id).await.is_none() {
            self.0.lock().await.insert(
                run_id.clone(),
                Declared {
                    datasets,
                    labels,
                    extraction,
                    hooks,
                },
            );
        }
    }

    /// Records the declarations kept for the resuming `run`, if any, in its
    /// run file, and answers the Hooks in code kept with them, none when
    /// nothing was kept. A write that fails is logged: the Run goes on
    /// under the declarations it held.
    pub(crate) async fn record(&self, run: &Run) -> Vec<CrawlHookDeclaration> {
        let Some(Declared {
            datasets,
            labels,
            extraction,
            hooks,
        }) = self.0.lock().await.remove(run.id())
        else {
            return Vec::new();
        };
        if let Err(error) = run
            .state()
            .write(move |connection| {
                datasets.record_again(connection)?;
                labels.record_labels_again(connection)?;
                code_free::record_again(connection, extraction.as_ref())
            })
            .await
        {
            tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's Datasets and Labels were not recorded again");
        }
        hooks
    }
}
