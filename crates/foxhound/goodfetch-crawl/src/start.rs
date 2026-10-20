//! `crawl.start` and Foxhound's crawl contribution (ADRs 0087, 0093 and
//! 0102): the start begins a crawl's Run through the engine's Runs, its
//! seeds written as its first work items in the transaction that records
//! the Run, and hands the Run to its delivery on the starting Connection's
//! Client. A Run ID that names a crawl of the same seeds returns it as it
//! is; one of other seeds, or of a Run that is no crawl, fails with
//! `run_conflict`.

use std::collections::BTreeMap;

use goodfetch_engine::run::{run_id, start_error};
use goodfetch_engine::runs::{EndedBy, Start, Started};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{CRAWL_START, CrawlStartParams, Seed};
use goodfetch_protocol::run::{Lifecycle, RunKind, RunStarted, StartInput, StartedAs};

use crate::delivery;
use crate::policy::CrawlPolicy;
use crate::work_items;

/// What Foxhound's crawl adds to the engine, under the default policy.
pub fn contribution() -> Contribution {
    contribution_with(CrawlPolicy::default())
}

/// What Foxhound's crawl adds to the engine, its attempts under `policy`:
/// `crawl.start`, and the work items' and Records' tables of every run
/// file.
pub fn contribution_with(policy: CrawlPolicy) -> Contribution {
    Contribution::new("goodfetch-crawl")
        .with_handler(Handler::new(
            &CRAWL_START,
            move |params: CrawlStartParams, context: CallContext| start(params, context, policy),
        ))
        .with_run_state_tables(work_items::TABLES)
}

async fn start(
    params: CrawlStartParams,
    context: CallContext,
    policy: CrawlPolicy,
) -> Result<RunStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
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
                policy,
            ));
            started
        }
        Started::Existing(existing) => existing,
    })
}

#[cfg(test)]
mod tests;
