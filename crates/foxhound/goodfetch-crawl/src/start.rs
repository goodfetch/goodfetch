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
//! The crawl's Datasets, refused with `dataset_duplicate` or
//! `dataset_key_invalid` before the Run begins, its Labels and
//! `crawl.payload.max_size` are recorded with the Run, and a seed whose Payload does not fit them fails the start with
//! `payload_invalid` before the Run begins; under a Run ID that names a
//! Run, whose seeds are not checked again, a Label whose Payload schema
//! changed under a pending Payload that fails it refuses the start with
//! `payload_schema_changed` before any work (ADR 0082). A Label whose
//! settings can never be served is refused with `setting_not_applicable`
//! before the Run begins (ADRs 0110 and 0120), and which requests keep
//! their URLs' fragments is recorded with the Run, so a start under its ID
//! that changes `crawl.keep_url_fragment` is refused with
//! `setting_not_applicable`. Such a start that drops a Dataset holding
//! Records is refused with `dataset_unknown`, and one that changes its key
//! with `dataset_key_invalid` (ADR 0103); its declarations, a code-free
//! crawl's extraction among them, are recorded when the Run resumes.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::{Began, Datasets};
use goodfetch_engine::run::{checkpoint_interval, run_error, run_id, settings_error, start_error};
use goodfetch_engine::runs::{Driving, EndedBy, RunError, Start, Started};
use goodfetch_engine::settings::Opening;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{CRAWL_SERVE, CRAWL_START, CrawlStartParams, Seed};
use goodfetch_protocol::foxhound::frontier::PAYLOAD_SCHEMA_CHANGED;
use goodfetch_protocol::foxhound::settings::{
    FEATURE_SWITCHED_OFF, PROVENANCE_LOG, SWITCHED_OFF_FEATURES,
};
use goodfetch_protocol::run::{
    Lifecycle, RUN_NOT_FOUND, RunDetails, RunId, RunKind, RunParams, RunReport, RunStarted,
    StartInput, StartedAs,
};
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_RUN};

use crate::code_free::{self, CodeFree};
use crate::delivery;
use crate::enqueue;
use crate::invariants::Invariants;
use crate::labels;
use crate::outcomes;
use crate::policy::domain_policy;
use crate::push;
use crate::redeclare::Redeclared;
use crate::retry;
use crate::run_state;
use crate::seeding;
use crate::servable;
use crate::serving::Crawls;
use crate::tier_choice::{self, TierChoices};
use crate::work_items;

/// What Foxhound's crawl adds to the engine: `crawl.start`, `crawl.serve`,
/// a handler's enqueue and push, the seeds added to a running crawl, the reads of
/// its outcomes, the re-admission of its failures, the work items', Records' and outcomes' tables of every
/// run file, the driver a resumed crawl starts again with, and the Audit
/// record of each per-page feature a Run switches off (ADR 0138). Its crawl
/// requests are fetched on the tiers of `tiers` (spec #579), the value
/// Foxhound's composition also hands Collie's pages.
pub fn contribution(tiers: FetchTiers) -> Contribution {
    let crawls = Crawls::default();
    let starting = tiers.clone();
    let serving = crawls.clone();
    let enqueuing = crawls.clone();
    let pushing = crawls.clone();
    let retrying = crawls.clone();
    let resuming = crawls.clone();
    let redeclared = Redeclared::default();
    let redeclaring = redeclared.clone();
    let pages = code_free::Pages::default();
    let [read_outcomes, read_failed_requests] = outcomes::handlers();
    let [add_seed, add_seeds] = seeding::handlers(&crawls);
    let [read_links, enqueue_links] = enqueue::links::handlers(&crawls, &tiers);
    // A per-page feature switched off is recorded once per Run (ADR 0138).
    let contribution = SWITCHED_OFF_FEATURES.into_iter().fold(
        Contribution::new("goodfetch-crawl"),
        |contribution, setting| {
            contribution.with_recorded_weakening(setting, &FEATURE_SWITCHED_OFF, setting.default)
        },
    );
    contribution
        .with_handler(Handler::new(&CRAWL_START, move |params, context| {
            start(
                params,
                context,
                crawls.clone(),
                starting.clone(),
                redeclared.clone(),
                pages.clone(),
            )
        }))
        .with_handler(Handler::new(&CRAWL_SERVE, move |params, context| {
            serve(params, context, serving.clone())
        }))
        .with_handler(enqueue::handler(enqueuing))
        .with_handler(push::handler(pushing))
        .with_handler(read_links)
        .with_handler(enqueue_links)
        .with_handler(add_seed)
        .with_handler(add_seeds)
        .with_handler(read_outcomes)
        .with_handler(read_failed_requests)
        .with_handler(retry::handler(retrying))
        .with_handler(crate::dataset_list::handler())
        .with_run_state_tables(work_items::TABLES)
        .with_run_state_tables(labels::TABLES)
        .with_run_state_tables(code_free::TABLES)
        .with_run_state_tables(tier_choice::TABLES)
        .with_report_part(Arc::new(crate::report::CrawlParts))
        .with_report_part(Arc::new(crate::completeness::StoppedParts))
        .with_progress_part(Arc::new(crate::progress::CrawlProgress))
        .with_driver(
            RunKind::Crawl,
            Arc::new(move |run, context, settings| {
                let (redeclared, crawls, tiers) =
                    (redeclaring.clone(), resuming.clone(), tiers.clone());
                Box::pin(async move {
                    redeclared.record(&run).await;
                    delivery::resume(run, context, settings, crawls, tiers).await;
                })
            }),
            DRIVING,
        )
}

/// How a crawl goes, which the engine learns with its driver (ADR 0099):
/// its driver ends it once its work is done, its claims defer their charges
/// (ADR 0138's note of #1926), so its start declares their bound (#2117),
/// `provenance.log` says whether it keeps a Provenance log, and its delivery
/// opens and serves its admission, which holds a Politeness of the Run's own
/// (#2171).
const DRIVING: Driving = Driving {
    ended_by: EndedBy::Driver,
    defers_charges: true,
    provenance_log: Some(&PROVENANCE_LOG),
    admits: true,
};

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
        .map_err(|error| run_error(error, &context))
}

/// A start's inputs: its `seeds`.
fn seed_inputs(seeds: &[Seed]) -> BTreeMap<StartInput, serde_json::Value> {
    BTreeMap::from([(
        StartInput::Seeds,
        serde_json::to_value(seeds).expect("invariant: seeds encode as JSON"),
    )])
}

async fn start(
    mut params: CrawlStartParams,
    context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
    redeclared: Redeclared,
    pages: code_free::Pages,
) -> Result<RunStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
    let work = context.runs().metrics().counted_work().clone();
    let pages = code_free::declare(&mut params, &pages, &work).await?;
    let (settings, _) = context
        .engine()
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
        .map_err(|error| settings_error(&error, &context))?;
    let label_settings = goodfetch_fetch_tier::LabelSettings::of(&params.labels)?;
    servable::check(settings.resolved(), &label_settings)?;
    let fragments = labels::Fragments::of(settings.resolved(), &label_settings);
    let labels = labels::Declared::of(&params.labels, fragments.clone(), settings.resolved())?;
    labels.sends_from_login_pools(settings.resolved())?;
    let datasets = pages.map_or_else(|| Datasets::declare(&params.datasets), Ok)?;
    FetchTiers::check(settings.resolved(), &labels.settings)?;
    let invariants = Invariants::of_settings(settings.resolved(), &params.labels, fragments)?;
    let egress = domain_policy(settings.resolved())?;
    let found = schema_unchanged(&context, run_id.as_ref(), &invariants, &datasets).await?;
    let seeding = egress.clone();
    // Each seed is keyed in the write that adds it, as its gate will key
    // it, so the crawl opens with no work item to key (#2168).
    let keys = context.runs().keys(settings.resolved());
    let seeds: Vec<Seed> = params.seeds.iter().cloned().collect();
    let inputs = seed_inputs(&seeds);
    // A Run that exists goes on with its seeds admitted once (ADR 0102).
    let checked = if found {
        Vec::new()
    } else {
        work_items::check_seeds(&invariants, seeds, &work)?
    };
    let declared = params.labels;
    let redeclaring = (datasets.clone(), params.extraction.clone());
    let opening = datasets.clone();
    let serving = invariants.clone();
    let admitted = code_free::admitted(params.extraction.as_ref(), checked);
    let (recording, extraction) = (params.extraction.clone(), params.extraction);
    let started = context
        .runs()
        .start(Start {
            run_id,
            tags: params.tags,
            kind: RunKind::Crawl,
            needs: Product::Foxhound,
            inputs,
            ended_by: DRIVING.ended_by,
            initialize: Some(Box::new(move |connection| {
                invariants.record(connection)?;
                labels::record(connection, &declared)?;
                datasets.record(connection)?;
                code_free::record(connection, recording.as_ref())?;
                let mut positions = match keys {
                    Some(keys) => work_items::Positions::keying(connection, keys)?,
                    None => work_items::Positions::of(connection)?,
                };
                let answers = run_state::admission::admit_seeds(
                    connection,
                    &mut positions,
                    (&seeding, &invariants),
                    &admitted,
                )?;
                goodfetch_frontier::seeds::answered(connection, &answers)
            })),
            policy_version: settings.resolved().policy_version().map(str::to_owned),
            weakened: settings.resolved().weakened(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().get(&TIMEOUT_RUN),
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
            let code_free = CodeFree::of(extraction, &crawls, &tiers, &run);
            run.spawn(delivery::drive(
                run.clone(),
                context.clone(),
                crawls.open(&run, egress, serving, opening).await,
                crawls,
                settings,
                (tiers, labels, TierChoices::default(), code_free),
            ));
            started
        }
        Started::Existing(existing) => redeclared.keep(existing, redeclaring, &crawls).await,
    })
}

/// Whether a Run `run_id` exists (never when the start names none), which
/// a start under its ID goes on with (ADR 0102), its seeds not admitted
/// again, once it may go on under the
/// crawl's `invariants`: refused with `payload_schema_changed` before any
/// work when a declared Label's Payload schema changed and a pending
/// Payload of it fails the new one (ADR 0082), with
/// `setting_not_applicable` when the Labels' fragments differ from those
/// the Run began with (ADR 0120), and with `dataset_unknown` or
/// `dataset_key_invalid` when `datasets` drop or key anew a Dataset that
/// holds Records (ADR 0103).
async fn schema_unchanged(
    context: &CallContext,
    run_id: Option<&RunId>,
    invariants: &Invariants,
    datasets: &Datasets,
) -> Result<bool, CallError> {
    let Some(run_id) = run_id else {
        return Ok(false);
    };
    let declared = invariants.payloads().clone();
    let failed = |error: RunError| run_error(error, context);
    let mut reader = match context.runs().reader(run_id.clone()).await {
        Ok(reader) => reader,
        Err(RunError::NotFound(_)) => return Ok(false),
        Err(error) => return Err(failed(error)),
    };
    let changed = match reader
        .read(move |connection| {
            let began = labels::Fragments::read(connection)?;
            let changed = declared.schema_changed(connection, |label| {
                work_items::pending_of(connection, label)
            })?;
            Ok((began, Began::read(connection)?, changed))
        })
        .await
    {
        Ok((began, began_with, changed)) => {
            invariants.fragments().unchanged(&began, run_id.as_str())?;
            datasets.resumes(&began_with, run_id.as_str())?;
            changed
        }
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
