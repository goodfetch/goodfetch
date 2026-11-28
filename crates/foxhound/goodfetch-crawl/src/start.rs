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

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::{Began, Datasets};
use goodfetch_engine::admission::Keys;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run::{
    InProgram, StartLimits, checkpoint_interval, client_start, resume_run, run_error, run_id,
    run_layer, settings_error, start_error,
};
use goodfetch_engine::runs::{Driving, EndedBy, Initialize, RunError, Start, Started};
use goodfetch_engine::settings::{Opening, RunSettings};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_SERVE, CRAWL_START, CrawlStartParams, LabelDeclaration, Seed,
};
use goodfetch_protocol::foxhound::frontier::PAYLOAD_SCHEMA_CHANGED;
use goodfetch_protocol::foxhound::hook::CrawlHookDeclaration;
use goodfetch_protocol::foxhound::settings::{
    FEATURE_SWITCHED_OFF, PROVENANCE_LOG, SWITCHED_OFF_FEATURES,
};
use goodfetch_protocol::run::{
    Lifecycle, RUN_NOT_FOUND, RunDetails, RunId, RunKind, RunParams, RunReport, RunStarted,
    StartInput, StartedAs,
};
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_RUN};

use crate::code_free::{self, CodeFree, CodeFreeCrawl};
use crate::crawl_run::CrawlRun;
use crate::delivery;
use crate::enqueue;
use crate::hooks;
use crate::invariants::Invariants;
use crate::labels;
use crate::outcomes;
use crate::partition::{MembershipWriter, PartitionRouter, PartitionStartOptions};
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

/// Shared runtime handle for Foxhound's crawl contribution, allowing
/// `goodfetch-partitioned-crawl` to start Partition Runs, route forwarded links
/// through Run-state admission, flush Run-state batches before handoff, and
/// re-attach Partition routers on resume (ADRs 0054, 0087 and 0140).
#[derive(Clone)]
pub struct CrawlRuntime {
    crawls: Crawls,
    tiers: FetchTiers,
    redeclared: Redeclared,
    pages: code_free::Pages,
}

impl CrawlRuntime {
    /// Creates a new crawl runtime bound to `tiers`.
    #[must_use]
    pub fn new(tiers: FetchTiers) -> Self {
        Self {
            crawls: Crawls::default(),
            tiers,
            redeclared: Redeclared::default(),
            pages: code_free::Pages::default(),
        }
    }

    /// Starts one Partition Run of a Partitioned crawl with its own
    /// [`PartitionStartOptions`] (`extra_limit_scopes`, `PartitionRouter`,
    /// membership persistence hook, and optional empty initial seeds).
    ///
    /// # Errors
    ///
    /// Any start refusal from `crawl.start`.
    pub async fn start_partition(
        &self,
        params: CrawlStartParams,
        initial_seeds: Vec<Seed>,
        options: PartitionStartOptions,
        context: CallContext,
    ) -> Result<RunStarted, CallError> {
        start_with_options(
            params,
            Some(initial_seeds),
            Some(options),
            context,
            self.crawls.clone(),
            self.tiers.clone(),
            self.redeclared.clone(),
            self.pages.clone(),
        )
        .await
    }

    /// Admits links forwarded from another Partition through this Partition's
    /// Run-state admission with its Run invariants (one actor round trip,
    /// deduplicated by its Frontier; ADR 0140, F3).
    ///
    /// # Errors
    ///
    /// `run_not_found` when `run_id` is not live on this engine, or any
    /// admission error.
    pub async fn admit_forwarded(
        &self,
        run_id: &RunId,
        seeds: Vec<Seed>,
    ) -> Result<usize, CallError> {
        let Some(live) = self.crawls.live(run_id).await else {
            return Err(CallError::new(
                &RUN_NOT_FOUND,
                format!("Partition Run `{run_id}` is not live on this Group member."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ));
        };
        let count = seeds.len();
        match live.asking.seeds(seeds).await {
            Ok(Some(Ok(_))) => Ok(count),
            Ok(Some(Err(refused))) => Err(refused),
            Ok(None) => Ok(0),
            Err(error) => Err(CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("failed to admit forwarded links into `{run_id}`: {error}"),
            )),
        }
    }

    /// Flushes the current Run-state batch of live Partition `run_id` (required
    /// before a Partition handoff, 2026-11-07 Foxhound refactor amendment).
    ///
    /// # Errors
    ///
    /// `run_not_found` when not live, or a run-state flush error.
    pub async fn flush_run_state(&self, run_id: &RunId) -> Result<(), CallError> {
        let Some(live) = self.crawls.live(run_id).await else {
            return Err(CallError::new(
                &RUN_NOT_FOUND,
                format!("Partition Run `{run_id}` is not live on this Group member."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ));
        };
        live.run.state.seal().await.map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("failed to flush Run state for `{run_id}`: {error}"),
            )
        })
    }

    /// Registers `router` for `run_id` so resuming `run_id` re-attaches it.
    pub fn register_router(&self, run_id: RunId, router: Arc<dyn PartitionRouter>) {
        self.crawls.register_router(run_id, router);
    }

    /// Returns the current run epoch of live Partition `run_id`, if live.
    pub async fn live_run_epoch(&self, run_id: &RunId) -> Option<u64> {
        self.crawls
            .live(run_id)
            .await
            .map(|live| live.run.state.epoch())
    }

    /// Returns the run folder of live Partition `run_id`, if live.
    pub async fn live_folder(&self, run_id: &RunId) -> Option<std::path::PathBuf> {
        self.crawls
            .live(run_id)
            .await
            .map(|live| live.run.identity.folder().to_path_buf())
    }

    /// Returns whether Partition `run_id` is currently live on this engine.
    pub async fn is_live(&self, run_id: &RunId) -> bool {
        self.crawls.live(run_id).await.is_some()
    }

    /// Fences `domains` on live Partition `run_id`, waits for their in-flight
    /// requests to end, seals the current Run-state batch, extracts their
    /// pending URLs, seen fingerprints and per-domain state, and removes them
    /// from `run_id` (ADR 0054 and ADR 0142).
    ///
    /// # Errors
    ///
    /// `run_not_found` when `run_id` is not live, or any Run-state error.
    pub async fn fence_and_drain_domains(
        &self,
        run_id: &RunId,
        domains: Vec<String>,
    ) -> Result<Vec<crate::partition::DomainHandoffSnapshot>, CallError> {
        let Some(live) = self.crawls.live(run_id).await else {
            return Err(CallError::new(
                &RUN_NOT_FOUND,
                format!("Partition Run `{run_id}` is not live on this Group member."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ));
        };
        live.asking
            .fence_and_drain_domains(domains)
            .await
            .map_err(|error| {
                CallError::protocol(
                    goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                    format!("failed to drain domains on `{run_id}`: {error}"),
                )
            })
    }

    /// Fences incoming `domains` on live Partition `run_id` until their
    /// handoff is acknowledged (ADR 0054).
    ///
    /// # Errors
    ///
    /// `run_not_found` when `run_id` is not live, or any Run-state error.
    pub async fn hold_incoming_domains(
        &self,
        run_id: &RunId,
        domains: Vec<String>,
    ) -> Result<(), CallError> {
        let Some(live) = self.crawls.live(run_id).await else {
            return Err(CallError::new(
                &RUN_NOT_FOUND,
                format!("Partition Run `{run_id}` is not live on this Group member."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ));
        };
        live.asking
            .hold_incoming_domains(domains)
            .await
            .map_err(|error| {
                CallError::protocol(
                    goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                    format!("failed to hold incoming domains on `{run_id}`: {error}"),
                )
            })
    }

    /// Installs one moving domain's pending URLs, seen fingerprints and
    /// per-domain state on live Partition `run_id` while keeping the domain
    /// fenced until unfenced after acknowledgement (ADR 0054).
    ///
    /// # Errors
    ///
    /// `run_not_found` when `run_id` is not live, or any Run-state error.
    pub async fn install_domain_handoff(
        &self,
        run_id: &RunId,
        snapshot: crate::partition::DomainHandoffSnapshot,
    ) -> Result<usize, CallError> {
        let Some(live) = self.crawls.live(run_id).await else {
            return Err(CallError::new(
                &RUN_NOT_FOUND,
                format!("Partition Run `{run_id}` is not live on this Group member."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ));
        };
        live.asking
            .install_domain_handoff(snapshot)
            .await
            .map_err(|error| {
                CallError::protocol(
                    goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                    format!("failed to install domain handoff on `{run_id}`: {error}"),
                )
            })
    }

    /// Lifts the handoff fence on `domains` on live Partition `run_id` once
    /// the handoff has been acknowledged (ADR 0054).
    ///
    /// # Errors
    ///
    /// `run_not_found` when `run_id` is not live, or any Run-state error.
    pub async fn unfence_domains(
        &self,
        run_id: &RunId,
        domains: Vec<String>,
    ) -> Result<(), CallError> {
        let Some(live) = self.crawls.live(run_id).await else {
            return Err(CallError::new(
                &RUN_NOT_FOUND,
                format!("Partition Run `{run_id}` is not live on this Group member."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ));
        };
        live.asking.unfence_domains(domains).await.map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("failed to unfence domains on `{run_id}`: {error}"),
            )
        })
    }
}

/// What Foxhound's crawl adds to the engine: `crawl.start`, `crawl.serve`,
/// a handler's enqueue and push, the seeds added to a running crawl, the reads of
/// its outcomes, the re-admission of its failures, the work items', Records' and outcomes' tables of every
/// run file, the driver a resumed crawl starts again with, and the Audit
/// record of each per-page feature a Run switches off (ADR 0138). Its crawl
/// requests are fetched on the tiers of `tiers` (spec #579), the value
/// Foxhound's composition also hands Collie's pages, and which finds the
/// Run of a raw call naming a page they serve (ADR 0099).
pub fn contribution(tiers: FetchTiers) -> Contribution {
    contribution_with(&CrawlRuntime::new(tiers))
}

/// Same as [`contribution`], sharing `runtime` with sibling Foxhound crates
/// such as `goodfetch-partitioned-crawl` (ADR 0087).
pub fn contribution_with(runtime: &CrawlRuntime) -> Contribution {
    let crawls = runtime.crawls.clone();
    let tiers = runtime.tiers.clone();
    let starting = tiers.clone();
    let serving = crawls.clone();
    let enqueuing = crawls.clone();
    let pushing = crawls.clone();
    let retrying = crawls.clone();
    let resuming = crawls.clone();
    let staging = crawls.clone();
    let redeclared = runtime.redeclared.clone();
    let (redeclaring, redeclared_retry) = (redeclared.clone(), redeclared.clone());
    let pages = runtime.pages.clone();
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
        .with_handler(enqueue::handler(enqueuing, tiers.clone()))
        .with_handler(push::handler(pushing))
        .with_handler(read_links)
        .with_handler(enqueue_links)
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
                    let run = CrawlRun::of(&run, &context);
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

/// A start's inputs: its `seeds`.
fn seed_inputs(seeds: &[Seed]) -> BTreeMap<StartInput, serde_json::Value> {
    BTreeMap::from([(
        StartInput::Seeds,
        serde_json::to_value(seeds).expect("invariant: seeds encode as JSON"),
    )])
}

async fn start(
    params: CrawlStartParams,
    context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
    redeclared: Redeclared,
    pages: code_free::Pages,
) -> Result<RunStarted, CallError> {
    start_with_options(
        params, None, None, context, crawls, tiers, redeclared, pages,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper uniting crawl.start and Partitioned crawl member starts without duplicating validation"
)]
async fn start_with_options(
    mut params: CrawlStartParams,
    override_seeds: Option<Vec<Seed>>,
    partition_options: Option<PartitionStartOptions>,
    context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
    redeclared: Redeclared,
    pages: code_free::Pages,
) -> Result<RunStarted, CallError> {
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
    let labels = labels::Declared::of(&params.labels, fragments.clone(), settings.resolved())?;
    labels.sends_from_login_pools(settings.resolved())?;
    let datasets = pages.map_or_else(|| Datasets::declare(&params.datasets), Ok)?;
    FetchTiers::check(settings.resolved(), &labels.settings)?;
    let (extra_limit_scopes, partition_router, membership_writer, initial_held_domains) =
        match partition_options {
            Some(opts) => (
                opts.extra_limit_scopes,
                Some(opts.router),
                opts.membership_writer,
                opts.initial_held_domains,
            ),
            None => (Vec::new(), None, None, Vec::new()),
        };
    let invariants = Invariants::of_settings(settings.resolved(), &params.labels, fragments)?
        .with_partition_router(partition_router.clone());
    let egress = domain_policy(settings.resolved())?;
    let found = schema_unchanged(&context, run_id.as_ref(), &invariants, &datasets).await?;
    let seeding = egress.clone();
    // Each seed is keyed in the write that adds it, as its gate will key
    // it, so the crawl opens with no work item to key (#2168).
    let keys = context.runs().keys(settings.resolved());
    let seeds: Vec<Seed> = override_seeds.unwrap_or_else(|| params.seeds.iter().cloned().collect());
    let inputs = seed_inputs(&seeds);
    // A Run that exists goes on with its seeds admitted once (ADR 0102).
    let checked = if found {
        Vec::new()
    } else {
        work_items::check_seeds(&invariants, seeds, &work)?
    };
    let (declared, opening, serving) = (params.labels, datasets.clone(), invariants.clone());
    let admitted = code_free::admitted(params.extraction.as_ref(), checked);
    let crawl = CodeFreeCrawl::of(params.extraction, params.declaration);
    let recording = crawl.clone();
    let hooks = params.hooks;
    let started = context
        .runs()
        .start_with_scopes(
            Start {
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
                    membership_writer,
                )),
                policy_version: settings.resolved().policy_version().map(str::to_owned),
                weakened: settings.resolved().weakened(),
                binding: None,
                clean_up_digest: None,
                checkpoint_interval: checkpoint_interval(settings.resolved()),
                timeout: settings.resolved().get(&TIMEOUT_RUN),
                settings: Some(settings.clone()),
                detached: params.detached,
            },
            extra_limit_scopes,
        )
        .await
        .map_err(start_error)?;
    finish_start(
        started,
        partition_router,
        initial_held_domains,
        (limits, settings, egress, serving, opening),
        (declared, crawl, hooks, labels),
        (crawls, tiers, redeclared, context),
    )
    .await
}

async fn finish_start(
    started: Started,
    partition_router: Option<Arc<dyn PartitionRouter>>,
    initial_held_domains: Vec<String>,
    (limits, settings, egress, serving, opening): (
        StartLimits,
        RunSettings,
        Policy,
        Invariants,
        Datasets,
    ),
    (declared, crawl, hooks, labels): (
        BTreeMap<String, LabelDeclaration>,
        Option<CodeFreeCrawl>,
        Vec<CrawlHookDeclaration>,
        labels::Declared,
    ),
    (crawls, tiers, redeclared, context): (Crawls, FetchTiers, Redeclared, CallContext),
) -> Result<RunStarted, CallError> {
    match started {
        Started::New(run) => {
            if let Some(router) = partition_router {
                crawls.register_router(run.identity.id().clone(), router);
            }
            let run = CrawlRun::of(&run, &context);
            let started = RunStarted {
                run_id: run.identity.id().clone(),
                started_as: StartedAs::New,
                lifecycle: Lifecycle::Running,
                needs: Product::Foxhound,
                partitions: Vec::new(),
            };
            // Its Limits hold it before its driver sends a request; a
            // crawl they cannot hold ends `failed` rather than run unheld.
            if let Err(error) = limits.give(&started, &context).await {
                run.end.end(delivery::failed(&run.identity, &error)).await;
                return Err(error);
            }
            let code_free = CodeFree::of(crawl, &crawls, &tiers, &run.identity);
            run.identity.spawn(delivery::drive(
                Arc::clone(&run),
                context.clone(),
                crawls
                    .open_with_held(&run, egress, serving, opening, initial_held_domains)
                    .await,
                crawls,
                settings,
                (tiers, labels, TierChoices::default(), (code_free, hooks)),
            ));
            Ok(started)
        }
        // An interrupted Run resumes under the declarations this start
        // checked (ADR 0102), its driver recording them.
        Started::Interrupted(run_id) => {
            let kept = (opening, declared, serving.payloads().clone(), crawl);
            // A crawl still ending the Run keeps nothing: the reopen is then
            // checked as no start's (ADR 0104's note of #2912).
            let kept = redeclared.keep(&run_id, kept, hooks, &crawls).await;
            let timeout = settings.resolved().get(&TIMEOUT_RUN);
            let resumed = resume_run(&context, run_id, None, settings, timeout, kept).await?;
            limits.give(&resumed, &context).await?;
            Ok(resumed)
        }
        Started::Ended(ended) => {
            let kept = (opening, declared, serving.payloads().clone(), crawl);
            redeclared.keep(&ended.run_id, kept, hooks, &crawls).await;
            limits.give(&ended, &context).await?;
            Ok(ended)
        }
    }
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
    membership_writer: Option<MembershipWriter>,
) -> Initialize {
    Box::new(move |connection| {
        invariants.record(connection)?;
        labels::record(connection, &declared)?;
        datasets.record(connection)?;
        code_free::record(connection, recording.as_ref())?;
        hooks::record(connection, &hooks)?;
        if let Some(writer) = membership_writer {
            writer(connection)?;
        }
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
    })
}

/// Whether a Run `run_id` exists (never when the start names none), which
/// a start under its ID goes on with (ADR 0102), its seeds not admitted
/// again, once it may go on under the
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
            let pending = work_items::pending_labels(connection)?;
            let changed = declared.schema_changed(connection, |label| {
                work_items::pending_of(connection, label)
            })?;
            Ok((began, Began::read(connection)?, pending, changed))
        })
        .await
    {
        Ok((began, began_with, pending, changed)) => {
            invariants.payloads().resumes(&pending, run_id.as_str())?;
            invariants.fragments().unchanged(&began, run_id)?;
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
