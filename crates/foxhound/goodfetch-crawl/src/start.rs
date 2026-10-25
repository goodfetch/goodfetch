//! `crawl.start` and Foxhound's crawl contribution (ADRs 0028, 0087, 0093
//! and 0102): the start resolves the Run's settings, refused before the Run
//! begins, begins a crawl's Run through the engine's Runs, its scope and
//! bounds recorded and each seed admitted by the Frontier, under the Run's
//! domain policy (`setting_invalid` for an entry that is no host), as its first work items,
//! and each seed's answer kept, in the transaction that records the Run, and hands the Run
//! to its delivery on the starting Connection's Client. A start whose
//! every seed was refused still starts, and its Run finishes at once; one
//! whose `fetch.tier` forces a tier this engine does not build is refused
//! with `capability_unavailable` (spec #579). A
//! Run ID that names a crawl of the same seeds returns it as it is; one of
//! other seeds, or of a Run that is no crawl, fails with `run_conflict`.
//! The crawl's Labels and `crawl.payload.max_size` are recorded with the
//! Run, and a seed whose Payload does not fit them fails the start with
//! `payload_invalid` before the Run begins; under a Run ID that names a
//! Run, whose seeds are not checked again, a Label whose Payload schema
//! changed under a pending Payload that fails it refuses the start with
//! `payload_schema_changed` before any work (ADR 0082).

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::run::{checkpoint_interval, run_error, run_id, settings_error, start_error};
use goodfetch_engine::runs::{EndedBy, RunError, Start, Started};
use goodfetch_engine::settings::Opening;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_fetch_tier::{FetchTiers, LabelSettings};
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_frontier::{Bounds, Scope};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{CRAWL_SERVE, CRAWL_START, CrawlStartParams, Seed};
use goodfetch_protocol::foxhound::frontier::PAYLOAD_SCHEMA_CHANGED;
use goodfetch_protocol::run::{
    Lifecycle, RUN_NOT_FOUND, RunDetails, RunId, RunKind, RunParams, RunReport, RunStarted,
    StartInput, StartedAs,
};
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_RUN};

use crate::delivery;
use crate::enqueue;
use crate::labels;
use crate::outcomes;
use crate::policy::domain_policy;
use crate::seeding;
use crate::serving::Crawls;
use crate::tier_choice::{self, TierChoices};
use crate::work_items;

/// What Foxhound's crawl adds to the engine: `crawl.start`, `crawl.serve`,
/// a handler's enqueue, the seeds added to a running crawl, the reads of
/// its outcomes, the work items', Records' and outcomes' tables of every
/// run file, and the driver a resumed crawl starts again with. Its crawl
/// requests are fetched on the tiers of `tiers` (spec #579), the value
/// Foxhound's composition also hands Collie's pages.
pub fn contribution(tiers: FetchTiers) -> Contribution {
    let crawls = Crawls::default();
    let starting = tiers.clone();
    let serving = crawls.clone();
    let enqueuing = crawls.clone();
    let resuming = crawls.clone();
    let [read_outcomes, read_failed_requests] = outcomes::handlers();
    let [add_seed, add_seeds] = seeding::handlers(&crawls);
    let [read_links, enqueue_links] = enqueue::links::handlers(&crawls, &tiers);
    Contribution::new("goodfetch-crawl")
        .with_handler(Handler::new(&CRAWL_START, move |params, context| {
            start(params, context, crawls.clone(), starting.clone())
        }))
        .with_handler(Handler::new(&CRAWL_SERVE, move |params, context| {
            serve(params, context, serving.clone())
        }))
        .with_handler(enqueue::handler(enqueuing))
        .with_handler(read_links)
        .with_handler(enqueue_links)
        .with_handler(add_seed)
        .with_handler(add_seeds)
        .with_handler(read_outcomes)
        .with_handler(read_failed_requests)
        .with_run_state_tables(work_items::TABLES)
        .with_run_state_tables(labels::TABLES)
        .with_run_state_tables(tier_choice::TABLES)
        .with_report_part(Arc::new(crate::report::CrawlParts))
        .with_report_part(Arc::new(crate::completeness::StoppedParts))
        .with_progress_part(Arc::new(crate::progress::CrawlProgress))
        .with_driver(
            RunKind::Crawl,
            Arc::new(move |run, context, settings| {
                Box::pin(delivery::resume(
                    run,
                    context,
                    settings,
                    resuming.clone(),
                    tiers.clone(),
                ))
            }),
        )
}

/// Joins the calling Connection's handler to the live crawl the params
/// name, until the Run ends, whose report it answers, or the call ends.
async fn serve(
    params: RunParams,
    context: CallContext,
    crawls: Crawls,
) -> Result<RunReport, CallError> {
    let Some(_stays) = crawls.join(&params.run_id, context.clone()).await else {
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
        .ended(params.run_id)
        .await
        .map_err(|error| run_error(error, context.identity()))
}

async fn start(
    params: CrawlStartParams,
    context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
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
            acting_for_model: context.surface().acts_for_model(),
        })
        .await
        .map_err(|error| settings_error(&error, context.identity()))?;
    let labels = LabelSettings::of(&params.labels)?;
    FetchTiers::check(settings.resolved(), &labels)?;
    let scope = Scope::of_settings(settings.resolved()).map_err(CallError::from)?;
    let bounds = Bounds::of_settings(settings.resolved());
    bounds.check().map_err(CallError::from)?;
    let egress = domain_policy(settings.resolved())?;
    let payloads = PayloadRules::of(settings.resolved(), &params.labels)?;
    let found = match &run_id {
        Some(run_id) => schema_unchanged(&context, run_id, payloads.clone()).await?,
        None => false,
    };
    if !found {
        for seed in params.seeds.iter() {
            payloads.check(&seed.label, seed.payload.as_ref())?;
        }
    }
    let seeding = egress.clone();
    let seeds: Vec<Seed> = params.seeds.iter().cloned().collect();
    let inputs = BTreeMap::from([(
        StartInput::Seeds,
        serde_json::to_value(&seeds).expect("invariant: seeds encode as JSON"),
    )]);
    let declared = params.labels;
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
                scope.record(connection)?;
                bounds.record(connection)?;
                payloads.record(connection)?;
                labels::record(connection, &declared)?;
                let answers = work_items::admit_seeds(connection, &scope, &seeding, &seeds)?;
                goodfetch_frontier::seeds::answered(connection, &answers)
            })),
            policy_version: settings.resolved().policy_version().map(str::to_owned),
            weakened: settings.resolved().weakened(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().duration(TIMEOUT_RUN.name),
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
            let opened = crawls.open(&run, egress).await;
            run.spawn(delivery::drive(
                run.clone(),
                context.clone(),
                opened,
                crawls,
                settings,
                (tiers, labels, TierChoices::default()),
            ));
            started
        }
        Started::Existing(existing) => existing,
    })
}

/// Whether a Run `run_id` exists, which a start under its ID goes on with
/// (ADR 0102), its seeds not admitted again, once it may go on under the
/// crawl's `declared` Labels: refused with `payload_schema_changed`
/// before any work when a Label's Payload schema changed and a pending
/// Payload of it fails the new one (ADR 0082).
async fn schema_unchanged(
    context: &CallContext,
    run_id: &RunId,
    declared: PayloadRules,
) -> Result<bool, CallError> {
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = match context.runs().reader(run_id.clone()).await {
        Ok(reader) => reader,
        Err(RunError::NotFound(_)) => return Ok(false),
        Err(error) => return Err(failed(error)),
    };
    let changed = match reader
        .read(move |connection| {
            declared.schema_changed(connection, |label| {
                work_items::pending_of(connection, label)
            })
        })
        .await
    {
        Ok(changed) => changed,
        Err(RunError::NotFound(_)) => return Ok(false),
        Err(error) => return Err(failed(error)),
    };
    match changed {
        None => Ok(true),
        Some(changed) => Err(CallError::new(
            &PAYLOAD_SCHEMA_CHANGED,
            format!(
                "The Label `{}`'s Payload schema changed since the Run `{run_id}` began, and {} of its {} pending Payloads fail it, the first that of crawl request `{}`, so the Run did no work: give the new field a default in the schema, or start a new Run.",
                changed.label, changed.failing, changed.pending, changed.crawl_request_id
            ),
            &changed,
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests;
