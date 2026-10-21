//! `crawl.start` and Foxhound's crawl contribution (ADRs 0028, 0087, 0093
//! and 0102): the start resolves the Run's settings, refused before the Run
//! begins, begins a crawl's Run through the engine's Runs, its seeds
//! written as its first work items in the transaction that records the
//! Run, and hands the Run to its delivery on the starting Connection's
//! Client. A Run ID that names a crawl of the same seeds returns it as it
//! is; one of other seeds, or of a Run that is no crawl, fails with
//! `run_conflict`.

use std::collections::BTreeMap;

use goodfetch_engine::run::{run_error, run_id, settings_error, start_error};
use goodfetch_engine::runs::{EndedBy, Start, Started};
use goodfetch_engine::settings::Opening;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{CRAWL_SERVE, CRAWL_START, CrawlStartParams, Seed};
use goodfetch_protocol::run::{
    Lifecycle, RUN_NOT_FOUND, RunDetails, RunKind, RunParams, RunReport, RunStarted, StartInput,
    StartedAs,
};
use goodfetch_protocol::settings::RunOrigin;

use crate::delivery;
use crate::outcomes;
use crate::serving::Crawls;
use crate::work_items;

/// What Foxhound's crawl adds to the engine: `crawl.start`, `crawl.serve`,
/// the reads of its outcomes, and the work items', Records' and outcomes'
/// tables of every run file.
pub fn contribution() -> Contribution {
    let crawls = Crawls::default();
    let serving = crawls.clone();
    let [read_outcomes, read_failed_requests] = outcomes::handlers();
    Contribution::new("goodfetch-crawl")
        .with_handler(Handler::new(&CRAWL_START, move |params, context| {
            start(params, context, crawls.clone())
        }))
        .with_handler(Handler::new(&CRAWL_SERVE, move |params, context| {
            serve(params, context, serving.clone())
        }))
        .with_handler(read_outcomes)
        .with_handler(read_failed_requests)
        .with_run_state_tables(work_items::TABLES)
}

/// Joins the calling Connection's handler to the live crawl the params
/// name, until the Run ends, whose report it answers, or the call ends.
async fn serve(
    params: RunParams,
    context: CallContext,
    crawls: Crawls,
) -> Result<RunReport, CallError> {
    let Some(_stays) = crawls.join(&params.run_id, context.client().clone()).await else {
        return Err(CallError::new(
            &RUN_NOT_FOUND,
            format!(
                "This engine runs no crawl `{}` to serve: it ended, or another engine runs it.",
                params.run_id
            ),
            &RunDetails {
                run_id: params.run_id,
                path: None,
            },
        ));
    };
    context
        .runs()
        .finish(params.run_id)
        .await
        .map_err(|error| run_error(error, context.identity()))
}

async fn start(
    params: CrawlStartParams,
    context: CallContext,
    crawls: Crawls,
) -> Result<RunStarted, CallError> {
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
            let joined = crawls.open(run.id().clone()).await;
            run.spawn(delivery::drive(
                run.clone(),
                context.client().clone(),
                joined,
                crawls,
                settings,
            ));
            started
        }
        Started::Existing(existing) => existing,
    })
}

#[cfg(test)]
pub(crate) mod tests;
