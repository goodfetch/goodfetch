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
//! `payload_invalid` before the Run begins, and a seed or a sitemap rule
//! naming a Label the crawl does not declare with `label_unknown` (ADR
//! 0093); under a Run ID that names a Run, whose seeds are not checked
//! again, a pending request of a Label the crawl no longer declares
//! refuses the start with `label_unknown`, and a Label whose Payload schema
//! changed under a pending Payload that fails it with
//! `payload_schema_changed`, before any work (ADRs 0082 and 0102). A Label whose
//! settings can never be served is refused with `setting_not_applicable`
//! before the Run begins (ADRs 0110 and 0120), and which requests keep
//! their URLs' fragments is recorded with the Run, so a start under its ID
//! that changes `crawl.keep_url_fragment` is refused with
//! `setting_not_applicable`. Such a start that drops a Dataset holding
//! Records is refused with `dataset_unknown`, and one that changes its key
//! with `dataset_key_invalid` (ADR 0103); its declarations, its Labels and
//! a code-free crawl's extraction among them, are recorded when the Run
//! resumes. A tag only the Daemon's scheduler sets is refused with
//! `invalid_params`, and a `detached` start of a crawl the program serves,
//! by its handler or a Hook, or on an engine that is no Daemon, with
//! `detach_unsupported`, before the Run begins; a code-free crawl started
//! detached on a Daemon outlives its Connection.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use goodfetch_dataset::{Began, Datasets};
use goodfetch_engine::admission::Keys;
use goodfetch_engine::audit::Origin;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run::{
    InProgram, StartLimits, checkpoint_interval, client_start, resume_run, run_error, run_id,
    run_layer, settings_error, start_error,
};
use goodfetch_engine::runs::{Driving, EndedBy, Initialize, RunError, Start, Started};
use goodfetch_engine::settings::{Opening, Resolved, RunSettings};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_KEEP_ALIVE, CRAWL_SERVE, CRAWL_START, CrawlStartParams, CrawlStarted, LabelDeclaration,
    Seed,
};
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_TRAP_LINKS_AVOID, Enqueued, PAYLOAD_SCHEMA_CHANGED,
};
use goodfetch_protocol::foxhound::hook::CrawlHookDeclaration;
use goodfetch_protocol::foxhound::settings::{
    FEATURE_SWITCHED_OFF, PROVENANCE_LOG, SWITCHED_OFF_FEATURES,
};
use goodfetch_protocol::run::warning::PERSONAL_DATA_NOTICE as PERSONAL_DATA_WARNING;
use goodfetch_protocol::run::{
    Lifecycle, RUN_NOT_FOUND, RunDetails, RunId, RunKind, RunParams, RunReport, RunStarted,
    RunWarning, StartInput, StartedAs,
};
use goodfetch_protocol::settings::{RunOrigin, SettingsLayer, TIMEOUT_RUN, WeakenedSetting};
use goodfetch_protocol::site_preferences::{
    PERSONAL_DATA_KEYWORD, PERSONAL_DATA_NOTICE as PERSONAL_DATA_AUDIT, SITE_PREFERENCES_PURPOSE,
};
use goodfetch_sink::Sinks;

use crate::code_free::{self, CodeFree, CodeFreeCrawl};
use crate::crawl_run::CrawlRun;
use crate::delivery;
use crate::enqueue;
use crate::hooks;
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
use crate::work_items::{self, CheckedSeed};

/// What Foxhound's crawl adds to the engine: `crawl.start`, `crawl.serve`,
/// a handler's enqueue and push, the seeds added to a running crawl, the reads of
/// its outcomes, the re-admission of its failures, the work items', Records' and outcomes' tables of every
/// run file, the driver a resumed crawl starts again with, and the Audit
/// record of each per-page feature a Run switches off (ADR 0138). Its crawl
/// requests are fetched on the tiers of `tiers` (spec #579), the value
/// Foxhound's composition also hands Collie's pages, and which finds the
/// Run of a raw call naming a page they serve (ADR 0099). Its crawls
/// deliver their Records to the Sinks of `sinks` (ADR 0026), the value of
/// Foxhound's composition.
pub fn contribution(tiers: FetchTiers, sinks: Sinks) -> Contribution {
    let crawls = Crawls::default();
    let starting = tiers.clone();
    let serving = crawls.clone();
    let enqueuing = crawls.clone();
    let pushing = crawls.clone();
    let retrying = crawls.clone();
    let resuming = crawls.clone();
    let staging = crawls.clone();
    let redeclared = Redeclared::default();
    let (redeclaring, redeclared_retry) = (redeclared.clone(), redeclared.clone());
    let pages = code_free::Pages::default();
    let [read_outcomes, read_failed_requests] = outcomes::handlers();
    let [add_seed, add_seeds] = seeding::handlers(&crawls);
    let [read_links, enqueue_links, enqueue_links_by_clicking] =
        enqueue::links::handlers(&crawls, &tiers);
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
                (starting.clone(), sinks),
                redeclared.clone(),
                pages.clone(),
            )
        }))
        .with_handler(Handler::new(&CRAWL_SERVE, move |params, context| {
            serve(params, context, serving.clone())
        }))
        .with_handler(enqueue::handler(enqueuing, tiers.clone()))
        .with_handler(push::handler(pushing))
        .with_handler(read_links)
        .with_handler(enqueue_links)
        .with_handler(enqueue_links_by_clicking)
        .with_handler(add_seed)
        .with_handler(add_seeds)
        .with_handler(read_outcomes)
        .with_handler(read_failed_requests)
        .with_handler(retry::handler(retrying, redeclared_retry))
        .with_handler(crate::dataset_list::handler())
        .with_run_state_tables(work_items::TABLES)
        .with_run_state_tables(labels::TABLES)
        .with_run_state_tables(code_free::TABLES)
        .with_run_state_tables(hooks::TABLES)
        .with_run_state_tables(tier_choice::TABLES)
        .with_report_part(Arc::new(crate::report::CrawlParts))
        .with_report_part(Arc::new(crate::completeness::StoppedParts))
        .with_progress_part(Arc::new(crate::progress::CrawlProgress))
        .with_run_of_call(Arc::new(tiers.clone()))
        .with_attempt_staging(Arc::new(staging))
        .with_driver(
            RunKind::Crawl,
            Arc::new(move |run, context, settings| {
                let (redeclared, crawls, tiers) =
                    (redeclaring.clone(), resuming.clone(), tiers.clone());
                Box::pin(async move {
                    // A resumed copy probes its Sinks again (ADR 0026): one
                    // that no longer meets the Run's minimum ends it.
                    let run =
                        match CrawlRun::started(&run, &context, (sinks, settings.resolved()))
                            .await
                        {
                            Ok(started) => started,
                            Err(error) => {
                                tracing::warn!(%error, run_id = %run.identity.id(), "the resumed crawl's Sinks were refused");
                                return;
                            }
                        };
                    let hooks = redeclared.record(&run.identity, &run.state).await;
                    // Boxed: the resumed delivery's future is large.
                    Box::pin(delivery::resume(
                        run,
                        context,
                        settings,
                        (crawls, tiers),
                        hooks,
                    ))
                    .await;
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
/// (#2171), and a reopen no start declared its Hooks in code again for is
/// refused when it has any (ADR 0104's note of #2912).
const DRIVING: Driving = Driving {
    ended_by: EndedBy::Driver,
    defers_charges: true,
    provenance_log: Some(&PROVENANCE_LOG),
    admits: true,
    reopen_check: Some(hooks::undeclared),
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

/// What gives the crawl `params` start a handler in the program, if
/// anything does: its program's crawl handler when it has neither
/// `extraction` nor `declaration`, else its Hooks in code (ADRs 0102 and
/// 0104).
fn in_program(params: &CrawlStartParams) -> Option<InProgram> {
    if params.extraction.is_none() && params.declaration.is_none() {
        return Some(InProgram {
            key: "extraction",
            fix: "Give the crawl an `extraction` or a `declaration`, so the engine handles each request itself, or start it without `detached`.",
        });
    }
    (!params.hooks.is_empty()).then_some(InProgram {
        key: "hooks",
        fix: "Start the crawl without its Hooks in code, `hooks`, or without `detached`.",
    })
}

/// A start's inputs: its `seeds`, and its `recording` when given.
fn seed_inputs(
    seeds: &[Seed],
    recording: Option<&goodfetch_protocol::run::Recording>,
) -> BTreeMap<StartInput, serde_json::Value> {
    let mut inputs = BTreeMap::from([(
        StartInput::Seeds,
        serde_json::to_value(seeds).expect("invariant: seeds encode as JSON"),
    )]);
    if let Some(recording) = recording {
        inputs.insert(
            StartInput::Recording,
            serde_json::to_value(recording).expect("invariant: a recording encodes as JSON"),
        );
    }
    inputs
}

async fn start(
    mut params: CrawlStartParams,
    context: CallContext,
    crawls: Crawls,
    (tiers, sinks): (FetchTiers, Sinks),
    redeclared: Redeclared,
    pages: code_free::Pages,
) -> Result<CrawlStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
    client_start(&params.tags, params.detached, in_program(&params), &context)?;
    let given = std::mem::take(&mut params.limits);
    let limits = StartLimits::checked(given, run_id.as_ref(), &context).await?;
    let work = context.runs().metrics().counted_work().clone();
    let pages = code_free::declare(&mut params, &pages, &work, &context).await?;
    let settings = opened_settings(&mut params, &context).await?;
    let label_settings = goodfetch_fetch_tier::LabelSettings::of(&params.labels)?;
    servable::check(settings.resolved(), &label_settings)?;
    let fragments = labels::Fragments::of(settings.resolved(), &label_settings);
    let avoid_trap_links = labels::TrapLinksAvoidance::of(settings.resolved(), &label_settings);
    let labels = labels::Declared::of(&params.labels, fragments.clone(), settings.resolved())?;
    labels.sends_from_login_pools(settings.resolved())?;
    let datasets = pages.map_or_else(|| Datasets::declare(&params.datasets), Ok)?;
    FetchTiers::check(settings.resolved(), &labels.settings)?;
    sinks.check(settings.resolved(), datasets.declarations())?;
    let invariants = Invariants::of_settings(
        settings.resolved(),
        &params.labels,
        fragments,
        avoid_trap_links.clone(),
    )?;
    let egress = domain_policy(settings.resolved())?;
    let existing_seeds =
        schema_unchanged(&context, run_id.as_ref(), &invariants, &datasets).await?;
    let seeding = egress.clone();
    // Each seed is keyed in the write that adds it, as its gate will key
    // it, so the crawl opens with no work item to key (#2168).
    let keys = context.runs().keys(settings.resolved());
    let seeds: Vec<Seed> = params.seeds.iter().cloned().collect();
    let record_path = params
        .recording
        .as_ref()
        .filter(|r| r.mode == goodfetch_protocol::run::RecordingMode::Record)
        .map(|r| r.path.clone());
    let inputs = seed_inputs(&seeds, params.recording.as_ref());
    let replayed = open_replay(params.recording.as_ref(), settings.resolved(), &context).await?;
    // A Run that exists goes on with its seeds admitted once (ADR 0102).
    let checked = if existing_seeds.is_some() {
        Vec::new()
    } else {
        work_items::check_seeds(&invariants, seeds, &work)?
    };
    let (declared, opening, serving) = (params.labels, datasets.clone(), invariants.clone());
    let admitted = code_free::admitted(params.extraction.as_ref(), checked);
    let crawl = CodeFreeCrawl::of(params.extraction, params.declaration);
    let recording = crawl.clone();
    let hooks = params.hooks;
    let new_answers: Arc<OnceLock<Vec<Enqueued>>> = Arc::default();
    let weakened = weakened_settings(settings.resolved(), &avoid_trap_links);
    let started = context
        .runs()
        .start(Start {
            run_id,
            tags: params.tags,
            kind: RunKind::Crawl,
            needs: Product::Foxhound,
            inputs,
            ended_by: DRIVING.ended_by,
            initialize: Some(initialize(
                invariants,
                (declared.clone(), datasets, recording, hooks.clone()),
                keys,
                seeding,
                admitted,
                Arc::clone(&new_answers),
            )),
            policy_version: settings.resolved().policy_version().map(str::to_owned),
            weakened,
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().get(&TIMEOUT_RUN),
            settings: Some(settings.clone()),
            detached: params.detached,
        })
        .await
        .map_err(start_error)?;
    finish_started(
        started,
        replayed,
        record_path,
        context,
        crawls,
        (tiers, sinks),
        redeclared,
        limits,
        settings,
        egress,
        (opening, declared, serving, crawl, labels, hooks),
        (new_answers, existing_seeds),
    )
    .await
}

async fn open_replay(
    recording: Option<&goodfetch_protocol::run::Recording>,
    settings: &goodfetch_engine::settings::Resolved,
    context: &CallContext,
) -> Result<Option<goodfetch_engine::response_recording::Replayed>, CallError> {
    let Some(recording) =
        recording.filter(|r| r.mode == goodfetch_protocol::run::RecordingMode::Replay)
    else {
        return Ok(None);
    };
    context
        .engine()
        .open_replay(&recording.path, settings)
        .await
        .map(Some)
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper splitting start's post-RunStarted branch"
)]
async fn finish_started(
    started: Started,
    replayed: Option<goodfetch_engine::response_recording::Replayed>,
    record_path: Option<String>,
    context: CallContext,
    crawls: Crawls,
    (tiers, sinks): (FetchTiers, Sinks),
    redeclared: Redeclared,
    limits: StartLimits,
    settings: RunSettings,
    egress: Policy,
    declarations: (
        Datasets,
        BTreeMap<String, LabelDeclaration>,
        Invariants,
        Option<CodeFreeCrawl>,
        labels::Declared,
        Vec<CrawlHookDeclaration>,
    ),
    seeds: (Arc<OnceLock<Vec<Enqueued>>>, Option<Vec<Enqueued>>),
) -> Result<CrawlStarted, CallError> {
    let (opening, declared, serving, crawl, labels, hooks) = declarations;
    let (new_answers, existing_seeds) = seeds;
    let software = format!(
        "{} {}",
        context.identity().product.name(),
        context.identity().product_version
    );
    match started {
        Started::New(run) => {
            if let Some(replayed) = replayed {
                context
                    .engine()
                    .recordings()
                    .attach(run.identity.id().clone(), replayed);
            } else if let Some(path) = record_path {
                let recorder =
                    context
                        .engine()
                        .new_recorder(path, run.identity.id().clone(), software, true);
                recorder.attach_run_state(run.state.clone());
                context
                    .engine()
                    .recordings()
                    .attach_recorder(run.identity.id().clone(), recorder);
            }
            start_new(
                (run, &context),
                (limits, settings, new_answers, sinks),
                (crawls, egress, serving, opening),
                (tiers, labels, crawl, hooks),
            )
            .await
        }
        // An interrupted Run resumes under the declarations this start
        // checked (ADR 0102), its driver recording them.
        Started::Interrupted(run_id) => {
            if let Some(replayed) = replayed {
                context
                    .engine()
                    .recordings()
                    .attach(run_id.clone(), replayed);
            } else if let Some(path) = record_path {
                let recorder = context
                    .engine()
                    .new_recorder(path, run_id.clone(), software, true);
                context
                    .engine()
                    .recordings()
                    .attach_recorder(run_id.clone(), recorder);
            }
            match resume_started(
                (&context, run_id.clone()),
                (settings, limits),
                (redeclared, &crawls),
                (opening, declared, serving.payloads().clone(), crawl),
                hooks,
                existing_seeds.unwrap_or_default(),
            )
            .await
            {
                Ok(resumed) => Ok(resumed),
                Err(error) => {
                    let _ = context.engine().recordings().take_report_of(&run_id);
                    Err(error)
                }
            }
        }
        Started::Ended(ended) => {
            let kept = (opening, declared, serving.payloads().clone(), crawl);
            redeclared.keep(&ended.run_id, kept, hooks, &crawls).await;
            limits.give(&ended, &context).await?;
            Ok(CrawlStarted {
                started: ended,
                seeds: existing_seeds.unwrap_or_default(),
            })
        }
    }
}

/// Shows ADR 0016's advisory personal-data notice once for a new crawl whose
/// declared Datasets mark one or more fields with `"x-goodfetch-personal":
/// true`, and records one `personal_data_notice` Audit log entry. Never blocks
/// or delays the Run.
async fn notify_personal_data(
    run: &CrawlRun,
    datasets: &Datasets,
    resolved: &Resolved,
    context: &CallContext,
) {
    let per_dataset = datasets.personal_fields();
    if per_dataset.is_empty() {
        return;
    }
    let fields: Vec<String> = per_dataset
        .iter()
        .flat_map(|(_, fields)| fields.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let purpose: &str = resolved.get(&SITE_PREFERENCES_PURPOSE);
    let joined = fields.join(", ");
    let notice = format!(
        "The Run's schema marks personal-data field(s) {joined} (`\"{PERSONAL_DATA_KEYWORD}\": true`; Purpose: `{purpose}`): review your data-protection obligations (including the CNIL measures for AI training when Purpose is `ai-train`). The Run is not blocked (ADR 0016)."
    );
    tracing::warn!(
        run_id = %run.identity.id(),
        fields = %joined,
        purpose = %purpose,
        "{notice}"
    );
    let warning = RunWarning {
        code: PERSONAL_DATA_WARNING.to_owned(),
        message: notice,
        sessions: Vec::new(),
        folder: None,
    };
    if let Err(error) = run.warnings.warn_once(warning).await {
        tracing::warn!(
            %error,
            run_id = %run.identity.id(),
            "the personal-data notice warning was not kept"
        );
    }
    let datasets_json: Vec<serde_json::Value> = per_dataset
        .into_iter()
        .map(|(dataset, fields)| serde_json::json!({ "dataset": dataset, "fields": fields }))
        .collect();
    let origin = Origin {
        actor: Actor::Engine,
        surface: Some(context.surface()),
        layer: None,
        run_id: Some(run.identity.id().clone()),
    };
    if let Err(error) = context
        .engine()
        .audit()
        .record(
            &origin,
            &PERSONAL_DATA_AUDIT,
            serde_json::json!({
                "fields": fields,
                "datasets": datasets_json,
                "purpose": purpose,
            }),
            run.identity.clock().timestamp(),
        )
        .await
    {
        tracing::error!(
            %error,
            run_id = %run.identity.id(),
            "the personal-data notice was not recorded in the Audit log"
        );
    }
}

/// Weakened settings of a crawl: the Run's own, plus `crawl.trap_links.avoid`
/// when a declared Label switches it off while the Run leaves it on.
fn weakened_settings(
    resolved: &Resolved,
    avoid_trap_links: &labels::TrapLinksAvoidance,
) -> Vec<WeakenedSetting> {
    let mut weakened = resolved.weakened();
    if avoid_trap_links.any_label_off()
        && !weakened
            .iter()
            .any(|setting| setting.key == CRAWL_TRAP_LINKS_AVOID.name)
    {
        weakened.push(WeakenedSetting {
            key: CRAWL_TRAP_LINKS_AVOID.name.to_owned(),
            value: serde_json::Value::Bool(false),
            layer: SettingsLayer::Run,
            origin: Some(RunOrigin::Settings),
            preset: None,
        });
    }
    weakened
}

/// Records `feature_switched_off` in the Audit log once for a new crawl when
/// a declared Label switches `crawl.trap_links.avoid` off while the Run
/// itself leaves it on (when the Run switches it off, the engine's recorded
/// weakening writes the entry).
async fn record_label_trap_links_off(
    run: &CrawlRun,
    labels: &labels::Declared,
    resolved: &Resolved,
    context: &CallContext,
) {
    let avoidance = labels::TrapLinksAvoidance::of(resolved, &labels.settings);
    if !resolved.get::<bool>(&CRAWL_TRAP_LINKS_AVOID) || !avoidance.any_label_off() {
        return;
    }
    let origin = Origin {
        actor: Actor::Engine,
        surface: Some(context.surface()),
        layer: Some(SettingsLayer::Run),
        run_id: Some(run.identity.id().clone()),
    };
    if let Err(error) = context
        .engine()
        .audit()
        .record(
            &origin,
            &FEATURE_SWITCHED_OFF,
            serde_json::json!({
                "setting": CRAWL_TRAP_LINKS_AVOID.name,
                "value": false,
                "default": (CRAWL_TRAP_LINKS_AVOID.default)(),
            }),
            run.identity.clock().timestamp(),
        )
        .await
    {
        tracing::error!(
            %error,
            run_id = %run.identity.id(),
            "the Label-level trap-links switch-off was not recorded in the Audit log"
        );
    }
}

/// Starts delivery for a newly created crawl Run after applying its Limits.
async fn start_new(
    (run, context): (goodfetch_engine::runs::RunParts, &CallContext),
    (limits, settings, new_answers, sinks): (
        StartLimits,
        RunSettings,
        Arc<OnceLock<Vec<Enqueued>>>,
        Sinks,
    ),
    (crawls, egress, serving, opening): (Crawls, Policy, Invariants, Datasets),
    (tiers, labels, crawl, hooks): (
        FetchTiers,
        labels::Declared,
        Option<CodeFreeCrawl>,
        Vec<CrawlHookDeclaration>,
    ),
) -> Result<CrawlStarted, CallError> {
    // A Sink below the Run's `crawl.min_sink_guarantee` refuses it
    // before it does any work (ADR 0026).
    let run = CrawlRun::started(&run, context, (sinks, settings.resolved())).await?;
    notify_personal_data(&run, &opening, settings.resolved(), context).await;
    record_label_trap_links_off(&run, &labels, settings.resolved(), context).await;
    let started = RunStarted {
        run_id: run.identity.id().clone(),
        started_as: StartedAs::New,
        lifecycle: Lifecycle::Running,
        needs: Product::Foxhound,
    };
    // Its Limits hold it before its driver sends a request; a
    // crawl they cannot hold ends `failed` rather than run unheld.
    if let Err(error) = limits.give(&started, context).await {
        run.end(delivery::failed(&run.identity, &error)).await;
        return Err(error);
    }
    let seeds = new_answers.get().cloned().unwrap_or_default();
    mark_initial_idle(&run, &settings, &seeds);
    let code_free = CodeFree::of(crawl, &crawls, &tiers, &run.identity);
    run.identity.spawn(delivery::drive(
        Arc::clone(&run),
        context.clone(),
        crawls.open(&run, egress, serving, opening).await,
        crawls,
        settings,
        (tiers, labels, TierChoices::default(), (code_free, hooks)),
    ));
    Ok(CrawlStarted { started, seeds })
}

/// Marks a newly started keep-alive crawl `idle: true` before its delivery task
/// runs when every initial seed was refused, so an immediate `run.progress()`
/// read sees `idle: true`.
fn mark_initial_idle(run: &CrawlRun, settings: &RunSettings, seeds: &[Enqueued]) {
    if settings.resolved().get::<bool>(&CRAWL_KEEP_ALIVE)
        && seeds
            .iter()
            .all(|answer| !matches!(answer, Enqueued::Added { .. }))
    {
        crate::progress::show_idle(run, true);
    }
}

/// Resumes an interrupted crawl under the declarations checked by `crawl.start`.
async fn resume_started(
    (context, run_id): (&CallContext, RunId),
    (settings, limits): (RunSettings, StartLimits),
    (redeclared, crawls): (Redeclared, &Crawls),
    kept: (
        Datasets,
        BTreeMap<String, LabelDeclaration>,
        goodfetch_frontier::payload::PayloadRules,
        Option<CodeFreeCrawl>,
    ),
    hooks: Vec<CrawlHookDeclaration>,
    seeds: Vec<Enqueued>,
) -> Result<CrawlStarted, CallError> {
    // A crawl still ending the Run keeps nothing: the reopen is then
    // checked as no start's (ADR 0104's note of #2912).
    let kept = redeclared.keep(&run_id, kept, hooks, crawls).await;
    let timeout = settings.resolved().get(&TIMEOUT_RUN);
    let resumed = resume_run(context, run_id, None, settings, timeout, kept).await?;
    limits.give(&resumed, context).await?;
    Ok(CrawlStarted {
        started: resumed,
        seeds,
    })
}

/// The crawl's settings, opened as a Foxhound Run's from the start's
/// `project` and `settings` layers.
async fn opened_settings(
    params: &mut CrawlStartParams,
    context: &CallContext,
) -> Result<RunSettings, CallError> {
    let (settings, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Foxhound),
            project: params.project.take(),
            recipe: None,
            settings: run_layer(&params.settings, context)?,
            origin: Some(RunOrigin::Settings),
            registered: context.registered_settings().clone(),
            acting_for_model: context.surface().acts_for_model(),
            exposed: context.exposed(),
        })
        .await
        .map_err(|error| settings_error(&error, context))?;
    Ok(settings)
}

/// A new crawl's first rows: its `invariants`, declared Labels, Datasets,
/// code-free extraction or declaration and Hooks in code, then its
/// `admitted` seeds, keyed with `keys` when its gates key work items and
/// under the `seeding` egress policy.
fn initialize(
    invariants: Invariants,
    (declared, datasets, recording, hooks): (
        BTreeMap<String, LabelDeclaration>,
        Datasets,
        Option<CodeFreeCrawl>,
        Vec<CrawlHookDeclaration>,
    ),
    keys: Option<Keys>,
    seeding: Policy,
    admitted: Vec<CheckedSeed>,
    kept_answers: Arc<OnceLock<Vec<Enqueued>>>,
) -> Initialize {
    Box::new(move |connection| {
        invariants.record(connection)?;
        labels::record(connection, &declared)?;
        datasets.record(connection)?;
        code_free::record(connection, recording.as_ref())?;
        hooks::record(connection, &hooks)?;
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
        goodfetch_frontier::seeds::answered(connection, &answers)?;
        let _ = kept_answers.set(answers);
        Ok(())
    })
}

/// Whether a Run `run_id` exists (never when the start names none), returning
/// its original seed answers when it does, which a start under its ID goes on
/// with (ADR 0102), its seeds not admitted again, once it may go on under the
/// crawl's `invariants`: refused with `label_unknown` before any work when
/// a pending request names a Label the crawl no longer declares (ADR
/// 0102), with `payload_schema_changed` when a declared Label's Payload
/// schema changed and a pending
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
) -> Result<Option<Vec<Enqueued>>, CallError> {
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    let declared = invariants.payloads().clone();
    let failed = |error: RunError| run_error(error, context);
    let mut reader = match context.runs().reader(run_id.clone()).await {
        Ok(reader) => reader,
        Err(RunError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    let (changed, seeds) = match reader
        .read(move |connection| {
            let began = labels::Fragments::read(connection)?;
            let pending = work_items::pending_labels(connection)?;
            let changed = declared.schema_changed(connection, |label| {
                work_items::pending_of(connection, label)
            })?;
            let seeds = goodfetch_frontier::seeds::read(connection)?;
            Ok((began, Began::read(connection)?, pending, changed, seeds))
        })
        .await
    {
        Ok((began, began_with, pending, changed, seeds)) => {
            invariants.payloads().resumes(&pending, run_id.as_str())?;
            invariants.fragments().unchanged(&began, run_id)?;
            datasets.resumes(&began_with, run_id.as_str())?;
            (changed, seeds)
        }
        Err(RunError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    match changed {
        None => Ok(Some(seeds)),
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
