//! `crawl.start` and Foxhound's crawl contribution (ADRs 0028, 0087, 0093
//! and 0102): the start resolves the Run's settings, refused before the Run
//! begins, begins a crawl's Run through the engine's Runs, its seeds
//! written as its first work items in the transaction that records the
//! Run, and hands the Run to its delivery on the starting Connection's
//! Client. A Run ID that names a crawl of the same seeds returns it as it
//! is; one of other seeds, or of a Run that is no crawl, fails with
//! `run_conflict`.

use std::collections::BTreeMap;

use goodfetch_engine::run::{run_id, settings_error, start_error};
use goodfetch_engine::runs::{EndedBy, Start, Started};
use goodfetch_engine::settings::Opening;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{CRAWL_START, CrawlStartParams, Seed};
use goodfetch_protocol::run::{Lifecycle, RunKind, RunStarted, StartInput, StartedAs};
use goodfetch_protocol::settings::RunOrigin;

use crate::delivery;
use crate::outcomes;
use crate::work_items;

/// What Foxhound's crawl adds to the engine: `crawl.start`, the reads of
/// its outcomes, and the work items', Records' and outcomes' tables of
/// every run file.
pub fn contribution() -> Contribution {
    let [read_outcomes, read_failed_requests] = outcomes::handlers();
    Contribution::new("goodfetch-crawl")
        .with_handler(Handler::new(&CRAWL_START, start))
        .with_handler(read_outcomes)
        .with_handler(read_failed_requests)
        .with_run_state_tables(work_items::TABLES)
}

async fn start(params: CrawlStartParams, context: CallContext) -> Result<RunStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
    let (settings, _) = context
        .runs()
        .settings()
        .open(Opening {
            run: Some(Product::Foxhound),
            project: params.project,
            recipe: None,
            settings: params.settings,
            origin: Some(RunOrigin::Settings),
        })
        .await
        .map_err(|error| settings_error(&error, context.identity()))?;
    let seeds: Vec<Seed> = params.seeds.iter().cloned().collect();
    let inputs = BTreeMap::from([(
        StartInput::Seeds,
        serde_json::to_value(&seeds).expect("invariant: seeds encode as JSON"),
    )]);
    let started = context
        .runs()
        .start(Start {
            run_id,
            tags: params.tags,
            kind: RunKind::Crawl,
            needs: Product::Foxhound,
            inputs,
            ended_by: EndedBy::Driver,
            initialize: Some(Box::new(move |connection| {
                work_items::insert_seeds(connection, &seeds)
            })),
            policy_version: settings.resolved().policy_version().map(str::to_owned),
            weakened_by: settings.resolved().weakened_by(),
            settings: Some(settings.clone()),
        })
        .await
        .map_err(start_error)?;
    Ok(match started {
        Started::New(run) => {
            let started = RunStarted {
                run_id: run.id().clone(),
                started_as: StartedAs::New,
                lifecycle: Lifecycle::Running,
                needs: Product::Foxhound,
            };
            run.spawn(delivery::drive(
                run.clone(),
                context.client().clone(),
                settings,
            ));
            started
        }
        Started::Existing(existing) => existing,
    })
}

#[cfg(test)]
pub(crate) mod tests;
