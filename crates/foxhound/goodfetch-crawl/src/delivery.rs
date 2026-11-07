//! A crawl's delivery (ADRs 0003, 0004, 0024 and 0093): the Run's driver,
//! which admits pending work items through the Run's admission, the
//! per-domain queues Foxhound's composition passes in as the gate in
//! admission under the Run's rate cap, and leases them to the handler of
//! each Connection serving the crawl (the one that started it, and each `workers=N` handler process
//! that joined it with `crawl.serve`), at most `crawl.handler.concurrency`
//! at once on each, each lease committed in run state
//! before its `crawl_request.handle` request is sent, served by a Session
//! the crawl's Session pool draws, which the attempt's ending then scores
//! (ADRs 0006 and 0021); commits an answered
//! attempt's Records once, through its lease, while the lease holds; and
//! ends an attempt whose handler failed, or that got no answer before its
//! lease ended, with nothing committed, its work item retried under the
//! Run's policy, which each admission point reads again from the Run's
//! settings; a handler's skip ends its work item `skipped`, and a failure
//! with the Error code `blocked` ends it `blocked`. Each ending's outcome
//! is committed with it. When a lease ends first, the attempt's request is cancelled
//! with `$/cancel`, and an answer that still comes is a late commit,
//! refused with `lease_lost`. An attempt whose Connection ended is
//! `client_gone`, never the handler's own `handler_failed`, and is retried
//! on a Connection still serving. Each admission also takes a Permit of its
//! domain key from the Politeness the Run shares with the other runs of
//! its user, given back once its attempt ended, with what the attempt met:
//! a site that did not answer in time backs the domain off, and a handler
//! failing with `blocked` for a `rate_limit` (a 429) halves its concurrency,
//! for every sharing run (ADR 0053), as one failing with
//! `response_unavailable` for a 502, 503 or 504 backs it off; the time a
//! failure's `retry_after` names holds the domain until then (ADR 0118).
//! The site's own answer to the request's document, which the engine
//! fetched on any tier, does the same whatever the handler made of the
//! page, as does a fetch the site refused, timed out or reset with
//! `ENHANCE_YOUR_CALM` (frontier's `backoff_classes`); a proxy's own
//! answer backs no domain off (ADR 0053). A
//! domain the Politeness holds for want of room waits for the wake of a
//! Permit given back. When that time is further away than the Run waits
//! (`politeness.retry_after.max`), the request whose response asked ends
//! `failed` with no retry spent, and each request of the domain admitted
//! meanwhile ends `failed` with `response_unavailable`, unsent: no handler
//! request goes for it. That time is measured from when the request's
//! handler request went out, and the Run refuses the domain until then,
//! so however late the failure arrives, nothing waits for it. Once every
//! work item is done, it finishes the Run; with `crawl.keep_alive` (RT-12) it waits
//! for new work instead until `run.finish` lets the Run finish, so its idle
//! time counts toward its `timeout.run`. A seed added to the running crawl
//! (`run.add_seed`, `run.add_seeds`) wakes it, idle or not, to key the new
//! work items and queue them under their domains. When the Run is asked to stop at its next safe point,
//! such as once its `timeout.run` passed, it admits nothing new, lets each
//! attempt in flight commit or be abandoned under its lease, and then stops
//! the Run. When the Run stops at once, by an abort, the attempts in flight
//! are abandoned and their work items return to pending without counting;
//! when
//! no Connection serves the crawl any more, the driver delivers nothing
//! more and waits for the Run to stop or a Connection to join. Each attempt
//! counts in `goodfetch.work_item.leased` until it ends, and each retry in
//! `goodfetch.work_item.retried`, on the meter the Run hands out (ADR 0050).
//! A pending work item a rule the Frontier learned later merged leaves its
//! domain's queue, and one the gate already took gives its turn back
//! unsent. Each attempt runs on the tier the crawl's tier choice plans for
//! its Label on its domain (`tier_choice`); a sample's outcome is
//! classified, and the choice written in the run file, as its attempt
//! settles, before the next admission.
//!
//! Size: one delivery loop, the admission, leases, attempts and settling it
//! drives, kept whole so every work item's way from pending to its ending
//! passes through one loop.

use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use futures_util::future::join_all;
use goodfetch_counted_work::Row;
use goodfetch_dataset::Datasets;
use goodfetch_engine::CallContext;
use goodfetch_engine::admission::{Admission, Ended, SecondCopies, Waiting, Wakes};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::ledger::Refusal;
use goodfetch_engine::local_addresses::{
    BlockSizes, LocalAddresses, Rotation, SessionAddresses, System, Unavailable,
};
use goodfetch_engine::mask::Scope;
use goodfetch_engine::metrics::Level;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::runs::{ChargeError, Ending, Run, UnjournaledCharges};
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_fetch_tier::{
    Choosing, ContentTypes, FetchAdmission, FetchTiers, HandOver, HttpTier, LocalSource,
    PooledSession, RunHandOver, SetTier, TierChoice, UntouchedDemand, UntouchedReaders,
};
use goodfetch_frontier::Bounds;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::egress::EGRESS_LOCAL_ADDRESSES;
use goodfetch_protocol::foxhound::crawl::{
    CLIENT_GONE, CRAWL_CONTENT_TYPES, CRAWL_KEEP_ALIVE, CrawlRequestHandled, HANDLER_FAILED,
    HANDLER_LOST, LEASE_LOST, WORK_ITEM_QUEUED, WORK_ITEM_RETRIED,
};
use goodfetch_protocol::foxhound::fetch::{
    FETCH_ENCODING, FETCH_SAMPLE_COUNT, FETCH_SAMPLE_REFRESH_AFTER, FETCH_TIER_RACE, FetchTier,
    SampleRefresh,
};
use goodfetch_protocol::foxhound::frontier::FRONTIER_LEARNING;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::report::{
    BLOCK_CLASSIFICATION_OFF, LEARNING_OFF, LOCAL_ADDRESS_ROTATED_WITHIN_BLOCK, PROVENANCE_OFF,
    RECORDS_OFF,
};
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::foxhound::settings::{
    DATASET_RECORDS, FETCH_BLOCK_CLASSIFICATION, PROVENANCE_LOG,
};
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::SITE_PREFERENCES_REFUSED;
use goodfetch_protocol::page::proxy::{PROXY_PROVIDER, PROXY_SERVER};
use goodfetch_protocol::run::{
    EndingError, NothingDone, RUN_STATE_UNAVAILABLE, RunDetails, RunWarning, StopReason,
};
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::usage::REQUESTS;
use goodfetch_session_pool::{Changes, LocalAddressing, PoolSettings, Rotated, SessionPool};
use serde_json::{Map, Value};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::attempt_outcome::{self, Outcome, Refusing};
use crate::code_free::{self, CodeFree};
use crate::completeness;
use crate::labels;
use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::{CrawlPolicy, resumed_domain_policy};
use crate::progress::{self, InFlight, Room, SharedInFlight};
use crate::serving::{Crawls, Opened, Server};
use crate::tier_choice::{self, AttemptPlan, Sampling, TierChoices, Unit};
use crate::tier_race::{Latencies, Raced};
use crate::work_items::{self, Failure, Leased, Unsuccessful};
use goodfetch_protocol::foxhound::progress::{HELD_DOMAINS, InFlightPhase, InFlightRequest};

mod attempt;
mod politeness;

use attempt::{
    Answer, Attempt, Taught, Tiering, attempt, block_cause, effect, handler_lease, leased_count,
    run_hand_over,
};
use politeness::{RETRY_AFTER_FIX, domain_ending, end_unsent, held_past_bound};

/// A Connection serving the crawl's handler requests, as the delivery
/// keeps it.
struct Serving {
    /// The context of the call that joined it, whose Connection's Client
    /// its handler requests go to.
    context: CallContext,
    /// Closed once it takes no new work item; `None` for the Connection
    /// that started the crawl, which serves until it ends.
    left: Option<oneshot::Receiver<()>>,
    /// Its attempts in flight.
    in_flight: usize,
    /// Whether its Connection ended.
    gone: bool,
}

impl Serving {
    /// A handler process that joined the crawl with `crawl.serve`.
    fn joined(server: Server) -> Self {
        Self {
            context: server.context,
            left: Some(server.left),
            in_flight: 0,
            gone: false,
        }
    }

    /// Whether it takes new work items.
    fn serves(&mut self) -> bool {
        !self.gone
            && self.left.as_mut().is_none_or(|left| {
                matches!(left.try_recv(), Err(oneshot::error::TryRecvError::Empty))
            })
    }
}

/// Drives `run`'s work items to the handler of `client`, and of each
/// Connection that `arrivals` brings, keying the seeds added since whenever
/// it says so, until each is done or the Run stops; then closes the crawl
/// in `crawls`.
///
/// # Cancel safety
///
/// Run it as one of the Run's tasks to its end: dropped, it leaves its
/// leases to the next writer, which counts them as lost.
pub(crate) async fn drive(
    run: Run,
    context: CallContext,
    mut arrivals: Opened,
    crawls: Crawls,
    settings: RunSettings,
    (tiers, labels, choices, code_free): (
        FetchTiers,
        labels::Declared,
        TierChoices,
        Option<CodeFree>,
    ),
) {
    let mut servers = vec![Serving {
        context,
        left: None,
        in_flight: 0,
        gone: false,
    }];
    // A code-free crawl fetches every request: with no tier set, the
    // engine chooses each, as under `auto` (ADR 0103).
    let run_tier = match FetchTiers::run_tier(settings.resolved()) {
        SetTier::Unset if code_free.is_some() => SetTier::Auto,
        set => set,
    };
    let races: bool = settings.resolved().get(&FETCH_TIER_RACE);
    // One HTTP tier for the Run, so its hops share connections (#763); the
    // navigation gate fetches robots.txt on it too, from the first
    // Session's local addresses under `egress.local_addresses` (ADR 0044).
    let interface = local_interface(settings.resolved());
    let built = match run_source(settings.resolved(), interface.as_ref()) {
        Ok(local) => tiers.built(&run, settings.resolved(), local).await,
        Err(error) => Err(error),
    };
    let http = match built {
        Ok(http) => Some(http),
        // A request may take the `http` tier: the Run cannot go on.
        Err(error) if labels.settings.any() || run_tier != SetTier::Unset || races => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's HTTP tier could not be built");
            run.end(failed(&run, &error)).await;
            crawls.close(run.id()).await;
            return;
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "the navigation gate has no transport for robots.txt; every robots.txt it reads is unreachable");
            None
        }
    };
    let untouched = Untouched::of(&run, &labels, code_free.as_ref());
    let fetching = Fetching {
        tiers: &tiers,
        http: http.as_ref(),
        interface,
        labels: std::sync::Arc::new(labels),
        run_tier,
        run_encoding: settings
            .resolved()
            .value(&FETCH_ENCODING)
            .as_str()
            .map(str::to_owned),
        // Its shape was checked when the crawl started.
        run_content_types: ContentTypes::of(settings.resolved().value(&CRAWL_CONTENT_TYPES))
            .unwrap_or_default(),
        choices: tokio::sync::Mutex::new(choices.sampled_as(sampling(settings.resolved()))),
        races,
        light: FetchTiers::light_allowed(settings.resolved()),
        latencies: tokio::sync::Mutex::new(Latencies::default()),
        in_flight: std::sync::Arc::new(tokio::sync::Mutex::new(InFlight::default())),
        untouched,
        code_free: code_free.map(std::sync::Arc::new),
    };
    for (setting, code, message) in SWITCHED_OFF {
        if !settings.resolved().get::<bool>(setting) {
            warn_switched_off(&run, code, message).await;
        }
    }
    // What every attempt hands over alike, built once (ADR 0138).
    let handing = run_hand_over(&run, fetching.http, &arrivals.egress).await;
    let fetching = (&fetching, &handing);
    deliver(&run, &mut servers, &mut arrivals, &settings, fetching).await;
    crawls.close(run.id()).await;
}

/// Each per-page feature a crawl switches off with its warning (ADR 0138):
/// its key, the warning's code and its message, so an empty result is not
/// misread.
const SWITCHED_OFF: [(&Setting, &str, &str); 4] = [
    (
        &FRONTIER_LEARNING,
        LEARNING_OFF,
        "`frontier.learning` is off: the Frontier signed no page and learned no URL rule, so `learned_rules` lists none.",
    ),
    (
        &PROVENANCE_LOG,
        PROVENANCE_OFF,
        "`provenance.log` is off: the Run trace holds no Provenance log; each Record still carries its own Provenance.",
    ),
    (
        &DATASET_RECORDS,
        RECORDS_OFF,
        "`dataset.records` is off: the crawl kept no Record, so its Datasets hold none.",
    ),
    (
        &FETCH_BLOCK_CLASSIFICATION,
        BLOCK_CLASSIFICATION_OFF,
        "`fetch.block_classification` is off: no document was read for a Challenge or a captcha, so a page a site blocked may be among the successes.",
    ),
];

/// Hands the Run's report, once, the warning `code` with `message`, of a
/// per-page feature its settings switch off (ADR 0138).
async fn warn_switched_off(run: &Run, code: &str, message: &str) {
    let warning = RunWarning {
        code: code.to_owned(),
        message: message.to_owned(),
        sessions: Vec::new(),
        folder: None,
    };
    if let Err(error) = run.warn_once(warning).await {
        tracing::warn!(%error, run_id = %run.id(), code, "the warning of a feature switched off was not kept");
    }
}

/// How the Run samples its tier choices: `fetch.sample.count` and
/// `fetch.sample.refresh_after`, the built-in where a value no longer
/// reads (the settings checked both at the start).
fn sampling(resolved: &Resolved) -> Sampling {
    let built_in = Sampling::default();
    let refresh = SampleRefresh::read(resolved.value(&FETCH_SAMPLE_REFRESH_AFTER)).ok();
    Sampling {
        count: resolved
            .value(&FETCH_SAMPLE_COUNT)
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(built_in.count),
        requests: refresh.map_or(built_in.requests, |refresh| refresh.requests),
        after_ms: refresh.map_or(built_in.after_ms, |refresh| refresh.after),
    }
}

/// Milliseconds since the Unix epoch at `run`'s wall clock.
fn wall_ms(run: &Run) -> u64 {
    run.clock()
        .wall()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Where `run`'s leases are granted, each attempt staging in the crawl's
/// staging, and its commit keying the work items it adds as `admission`'s
/// gate keys them.
fn leases_of(run: &Run, arrivals: &Opened, admission: &Admission) -> Leases {
    Leases::new(run.clock(), run.id().as_str(), run.state().epoch())
        .staging_in(arrivals.staging.clone())
        .checking_against(arrivals.datasets.clone())
        .counting_on(run.metrics().counted_work().clone())
        .keyed_by(admission.keys())
}

/// What the delivery's loop reads of the Run's settings, resolved once as
/// the Run opened: no admission, lease or settlement reads them again (ADR
/// 0138).
#[derive(Clone, Copy)]
struct Read {
    /// How its attempts are bounded and retried.
    policy: CrawlPolicy,
    /// The bounds its Session pool draws and scores under.
    pool: PoolSettings,
    /// Whether its requests go through a proxy.
    proxied: bool,
    /// The longest wait a response may ask of its domain.
    longest_wait: Option<Duration>,
}

impl Read {
    /// What `resolved` gives the loop.
    fn of(resolved: &Resolved) -> Self {
        Self {
            policy: CrawlPolicy::of(resolved),
            pool: PoolSettings::of(resolved),
            proxied: proxied(resolved),
            longest_wait: goodfetch_frontier::longest_wait(resolved),
        }
    }
}

/// The delivery's loop, which returns once the Run ended or stopped, each
/// attempt it starts handing over `handing` with its own part.
async fn deliver(
    run: &Run,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    settings: &RunSettings,
    (fetching, handing): (&Fetching<'_>, &std::sync::Arc<RunHandOver>),
) {
    // The pool is out of `pool` only while a write that leases work items
    // draws from it: the loop ends on that write's error.
    let Some((mut admission, mut pool)) = opened(run, settings).await else {
        return;
    };
    let run_leases = leases_of(run, arrivals, &admission);
    let (mut wakes, mut queued) = (admission.wakes(), run.metrics().level(&WORK_ITEM_QUEUED));
    let mut attempts: JoinSet<Attempt> = JoinSet::new();
    let mut stop: Option<StopReason> = None;
    // Resolved once, as the Run opened: no turn reads the settings again
    // (ADR 0138).
    let resolved = settings.resolved();
    let read = Read::of(resolved);
    let policy = read.policy;
    loop {
        if stop.is_none()
            && let Err(error) = lease_to_servers(
                run,
                &run_leases,
                policy,
                Pooled::of(&mut pool, read.pool, (fetching, handing)),
                &mut admission,
                servers,
                &mut attempts,
            )
            .await
        {
            tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be leased");
            end(run, &mut admission, failed(run, &error)).await;
            return;
        }
        let starved = run.governor().waits(WorkKind::Http);
        if attempts.is_empty() {
            if let Some(reason) = stop {
                end(run, &mut admission, Ending::Stopped(reason)).await;
                return;
            }
            if !servers.iter_mut().any(Serving::serves) {
                match waited(run, &mut admission, servers, arrivals, false).await {
                    Waited::Ended => return,
                    Waited::Stop(reason) => stop = Some(reason),
                    Waited::Again => {}
                }
                continue;
            }
            match when_idle(run, &mut admission).await {
                Ok(Idle::Admit) if !starved => continue,
                Ok(Idle::Admit | Idle::Wait) => {}
                Ok(Idle::Done) => {
                    match drained(run, resolved, &mut admission, servers, arrivals).await {
                        Waited::Ended => return,
                        Waited::Stop(reason) => stop = Some(reason),
                        Waited::Again => {}
                    }
                    continue;
                }
                Err(error) => {
                    tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                    end(run, &mut admission, failed(run, &error)).await;
                    return;
                }
            }
        }
        let free = stop.is_none()
            && servers
                .iter_mut()
                .any(|server| server.serves() && server.in_flight < policy.handler_concurrency);
        let wake = (free && !starved)
            .then(|| admission.ready_at(run.clock().now()))
            .flatten();
        let room = [stop.is_some(), free, starved];
        let shown = (resolved, policy);
        show(run, (&mut admission, &mut queued), shown, servers, room);
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => {
                attempts.shutdown().await;
                admission.close().await;
                release(run).await;
                return;
            }
            reason = run.stopping(), if stop.is_none() => {
                stop = Some(reason);
            }
            Some(server) = arrivals.joined.recv() => {
                servers.push(Serving::joined(server));
            }
            () = arrivals.seeded.notified() => {
                if let Err(error) = key_new(run, &mut admission).await {
                    tracing::warn!(%error, run_id = %run.id(), "the seeds added were not keyed yet");
                }
            }
            Some(first) = attempts.join_next() => {
                let ended = ended_with(first, &mut attempts);
                let pool = (held(&mut pool), fetching);
                settled(run, (resolved, read), &mut admission, pool, servers, ended).await;
            }
            () = sleep_until(run.clock(), wake), if wake.is_some() => {}
            () = run.governor().turn(WorkKind::Http), if free && starved => {}
            key = next_wake(wakes.as_mut()) => admission.wake(key.as_deref()),
        }
    }
}

/// The attempt that ended `first`, then every other of `attempts` that
/// ended by now, in the order they ended, so they settle together and the
/// loop's own work, its admission's included, runs once for them all
/// rather than once for each (#1765). An attempt whose task panicked or
/// was cancelled is left out, as when it settled alone.
fn ended_with(
    first: Result<Attempt, tokio::task::JoinError>,
    attempts: &mut JoinSet<Attempt>,
) -> Vec<Attempt> {
    std::iter::once(first)
        .chain(std::iter::from_fn(|| attempts.try_join_next()))
        .filter_map(Result::ok)
        .collect()
}

/// Sets the crawl's live parts of its Run's progress (ADR 0102) as its
/// `servers` stand, given whether it stops, whether a serving Connection
/// has room and whether the governor starves it, and its count in
/// `goodfetch.work_item.queued` as its admission's queues stand.
fn show(
    run: &Run,
    (admission, count): (&mut Admission, &mut Level),
    (resolved, policy): (&Resolved, CrawlPolicy),
    servers: &mut [Serving],
    [stopping, free, starved]: [bool; 3],
) {
    let serving = servers.iter_mut().any(Serving::serves);
    let handlers = servers.iter().map(|server| server.in_flight).sum();
    let room = progress::room(stopping || !serving, free, starved);
    let concurrency = policy.handler_concurrency;
    let now = run.clock().now();
    let queued = admission.queued(now, HELD_DOMAINS);
    count.set(admission.waiting());
    progress::show(run, (&queued, now), resolved, room, (handlers, concurrency));
}

/// The ending of a crawl whose state or admission failed with `error`:
/// `failed`, its ending calls raising `run_state_unavailable` (ADR 0102).
fn failed(run: &Run, error: &dyn std::fmt::Display) -> Ending {
    let details = RunDetails {
        run_id: run.id().clone(),
        path: Some(run.folder().display().to_string()),
    };
    Ending::Failed(EndingError {
        code: RUN_STATE_UNAVAILABLE.name.to_owned(),
        message: format!("The crawl `{}` failed: {error}", run.id()),
        details: match serde_json::to_value(details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        },
    })
}

/// Ends the Run as `ending` says, once its admission gave back what it held.
async fn end(run: &Run, admission: &mut Admission, ending: Ending) {
    admission.close().await;
    run.end(ending).await;
}

/// The next key of the Run that a Permit given back woke, `None` for every
/// held key; never, when the Run shares no Politeness.
async fn next_wake(wakes: Option<&mut Wakes>) -> Option<String> {
    match wakes {
        Some(wakes) => wakes.next().await,
        None => std::future::pending().await,
    }
}

/// Drives a resumed crawl (ADRs 0002 and 0102), its bounds recorded again
/// from its settings (an unreadable path budget fails it) and its Labels'
/// tier choices read again from its run file: each attempt its
/// committed state shows in flight, whose engine stopped without settling
/// it, counts as `handler_lost` against `crawl.retry.max` and is retried or
/// failed, then delivery goes on from the committed state, so the work
/// since that state is redone. Its domain policy is read again from its
/// settings; one that cannot be read admits no seed or enqueue.
///
/// # Cancel safety
///
/// As [`drive`]'s.
pub(crate) async fn resume(
    run: Run,
    context: CallContext,
    settings: RunSettings,
    crawls: Crawls,
    tiers: FetchTiers,
) {
    let policy = CrawlPolicy::of(settings.resolved());
    let retry_max = policy.retry_max;
    let bounds = Bounds::of_settings(settings.resolved());
    let resolved = settings.resolved().clone();
    let lost = Failure::new(HANDLER_LOST.name, HANDLER_LOST.description);
    let resumed_at = run.point_in_time();
    let left = run
        .state()
        .write(move |connection| {
            bounds.record(connection)?;
            goodfetch_frontier::payload::record_max_size(connection, &resolved)?;
            work_items::recover_leases(connection, &lost, retry_max, &resumed_at)?;
            Ok((
                work_items::left(connection)?,
                labels::read(connection)?,
                labels::Fragments::read(connection)?,
                tier_choice::read(connection)?,
                (code_free::read(connection)?, Datasets::read(connection)?),
            ))
        })
        .await;
    let left = left.map(|(left, declared, fragments, choices, declarations)| {
        let labels = labels::Declared::of(&declared, fragments).unwrap_or_else(|error| {
            tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's Labels no longer read; its requests take the Run's tier and pool");
            labels::Declared::default()
        });
        (left, (labels, choices, declarations))
    });
    let (labels, choices, (extraction, datasets)) = match left {
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the resumed crawl's work items could not be recovered");
            run.end(failed(&run, &error)).await;
            return;
        }
        Ok((0, labels)) => {
            if let Err(error) = run.nothing_done(NothingDone::NothingPending).await {
                tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's `nothingDone` was not kept");
            }
            labels
        }
        Ok((_, labels)) => labels,
    };
    let egress = resumed_domain_policy(settings.resolved(), run.id());
    let opened = crawls.open(&run, egress, datasets).await;
    let code_free = CodeFree::of(extraction, &crawls, &tiers, &run);
    drive(
        run,
        context,
        opened,
        crawls,
        settings,
        (tiers, labels, choices, code_free),
    )
    .await;
}

/// The tiers a Run's crawl requests are fetched on: its HTTP tier, and the
/// browser's pages through `tiers`, each request on the tier its Label and
/// the Run's `fetch.tier` choose.
struct Fetching<'a> {
    tiers: &'a FetchTiers,
    http: Option<&'a HttpTier>,
    /// The interface `egress.local_addresses` names, whose device each
    /// Session's sockets bind (ADR 0044).
    interface: Option<String>,
    /// The Labels the crawl declares: their settings and their pools.
    labels: std::sync::Arc<labels::Declared>,
    /// The tier the Run's own `fetch.tier` forces.
    run_tier: SetTier,
    /// The Run's own `fetch.encoding`, a WHATWG Encoding label.
    run_encoding: Option<String>,
    /// The Run's own `crawl.content_types`.
    run_content_types: ContentTypes,
    /// What each Label on each domain learned under `auto`.
    choices: tokio::sync::Mutex<TierChoices>,
    /// Whether the Run races its requests' tiers (`fetch.tier_race`).
    races: bool,
    /// Whether the Run allows the `light_browser` tier, which its samples
    /// then also run on (`fetch.light_browser`, ADR 0020).
    light: bool,
    /// The browser-fetch latencies of each Label on each domain, which
    /// time a race's navigation.
    latencies: tokio::sync::Mutex<Latencies>,
    /// What a code-free crawl handles its requests with; none when its
    /// program does.
    code_free: Option<std::sync::Arc<CodeFree>>,
    /// The requests in flight, as the Run's progress shows them.
    in_flight: SharedInFlight,
    /// The readers of the untouched page of each Label, built once.
    untouched: Untouched,
}

/// The readers of the untouched page a Run's documents are read by (ADR
/// 0138), built once as its crawl starts: those of the Labels that declare
/// a Page check of their own, and the Run's, for every other Label.
struct Untouched {
    run: std::sync::Arc<UntouchedReaders>,
    labels: BTreeMap<String, std::sync::Arc<UntouchedReaders>>,
}

impl Untouched {
    /// `run`'s readers, of its Labels `labels`: a code-free crawl's
    /// `code_free` links and fields, each Label's Page check, else the
    /// Run's, and the Page signature while the Run learns; a program's
    /// keep each page's tree for its handler.
    fn of(run: &Run, labels: &labels::Declared, code_free: Option<&CodeFree>) -> Self {
        let work = run.metrics().counted_work();
        goodfetch_fetch_tier::compile_served_call_selectors(work);
        let (links, fields) = code_free.map(CodeFree::untouched).unwrap_or_default();
        let demand = |page_check: Option<PageCheck>| UntouchedDemand {
            links: links.clone(),
            fields: fields.clone(),
            page_check,
            signs: attempt::learns(run),
            // A program's handler reads its pages: each keeps its untouched
            // tree for the handler's calls (spec #1577's #1918).
            keeps: code_free.is_none(),
        };
        let readers =
            |page_check| std::sync::Arc::new(UntouchedReaders::of(demand(page_check), work));
        Self {
            run: readers(attempt::run_page_check(run)),
            labels: labels
                .settings
                .page_checks()
                .map(|(label, check)| (label.to_owned(), readers(Some(check.clone()))))
                .collect(),
        }
    }

    /// The readers of a request of `label`.
    fn of_label(&self, label: &str) -> &std::sync::Arc<UntouchedReaders> {
        self.labels.get(label).unwrap_or(&self.run)
    }
}

impl Fetching<'_> {
    /// What the attempt of `leased`, holding `lease`, hands over with its
    /// Run's pages, built once as it is leased: `of_run`, what every
    /// attempt of the Run hands over alike, with its own room beside the
    /// Run's requests, `second_copies`, its HTTP admission and what its
    /// request's Label declares.
    fn hand_over(
        &self,
        of_run: &std::sync::Arc<RunHandOver>,
        (leased, lease): (&Leased, &WorkItemLease),
        second_copies: &SecondCopies,
        fetch_admission: FetchAdmission,
    ) -> std::sync::Arc<HandOver> {
        let label = leased.crawl_request.label.as_str();
        std::sync::Arc::new(HandOver {
            of_run: std::sync::Arc::clone(of_run),
            domain_key: leased.waiting.key.clone(),
            second_copies: second_copies.clone(),
            lease: handler_lease(&of_run.run, leased, lease, &self.in_flight),
            fetch_admission,
            page_check: self.labels.settings.page_check(label).cloned(),
            extractor: self.labels.settings.extractor(label),
            encoding: self
                .labels
                .settings
                .encoding(label, self.run_encoding.as_deref()),
            content_types: self
                .labels
                .settings
                .content_types(label, &self.run_content_types),
            untouched: std::sync::Arc::clone(self.untouched.of_label(label)),
            capture: None,
        })
    }

    /// How `leased`'s attempt runs, as `chosen` (its Label and the Run's
    /// `fetch.tier` chose it as its Session was drawn): a race is timed by
    /// its Label on its domain's browser-fetch latencies; and what it
    /// fetches its document with.
    async fn served(
        &self,
        leased: &Leased,
        (chosen, local): (Chosen, Local),
        second_copies: &SecondCopies,
    ) -> Tiering {
        let request = &leased.crawl_request;
        let plan = match chosen {
            Chosen::Planned(plan) => plan,
            // Until the sample comparison weighs a race's runs (#768), a
            // race learns nothing: a Label on its domain due a sample takes
            // it, so a racing Run learns its choice as any other (ADR
            // 0040's #945 note).
            Chosen::Racing(sample @ AttemptPlan::Sample(_)) => sample,
            Chosen::Racing(learned) => AttemptPlan::Race(Raced {
                label: request.label.clone(),
                domain_key: leased.waiting.key.clone(),
                delay: self
                    .latencies
                    .lock()
                    .await
                    .delay(&request.label, &leased.waiting.key),
                agrees: !matches!(
                    learned,
                    AttemptPlan::On(TierChoice {
                        tier: FetchTier::Browser,
                        ..
                    })
                ),
            }),
        };
        let http = self.session_tier(local);
        Tiering::of(
            self.tiers,
            http.as_ref(),
            (plan, self.light),
            self.labels.settings.page_check(&request.label).cloned(),
            second_copies,
        )
    }

    /// The Run's HTTP tier as a Session holding `local` sends on it (ADR
    /// 0044): `None`, which fails the attempt's fetch, when the Session's
    /// addresses are no longer held or its transport cannot be built,
    /// rather than sending from an address the OS chooses.
    fn session_tier(&self, local: Local) -> Option<HttpTier> {
        let http = self.http?;
        let addresses = match local {
            None => return Some(http.clone()),
            Some(Ok(addresses)) => addresses,
            Some(Err(unavailable)) => {
                tracing::warn!(entry = %unavailable.entry, "a Session's local addresses are no longer held; its HTTP fetches fail");
                return None;
            }
        };
        let source = LocalSource::new(addresses, self.interface.clone());
        http.from_session(Some(&source))
            .inspect_err(|error| tracing::warn!(%error, "a Session's HTTP transport could not be built; its HTTP fetches fail"))
            .ok()
    }

    /// Closes the browser contexts of the retired Sessions of `pool` no
    /// attempt holds any more, for the call `context`.
    async fn retire(&self, run: &Run, pool: &mut SessionPool, context: &CallContext) {
        for session in pool.closable() {
            if let Err(error) = self.tiers.retire_browser(run.id(), &session, context).await {
                tracing::error!(%error, run_id = %run.id(), session_id = %session.0, "a retired Session's browser context did not close");
            }
        }
    }

    /// Takes the browser-fetch latency of the race `attempt` was, if it
    /// was one and its browser-fetch answered.
    async fn time(&self, attempt: &mut Attempt) {
        if let Some((raced, latency)) = attempt.raced.take() {
            self.latencies
                .lock()
                .await
                .record(&raced.label, &raced.domain_key, latency);
        }
    }

    /// Classifies the sample `attempt` was, if it was one, answering what
    /// its Label on its domain now learned, by Label and domain key, for
    /// its outcome to write into the run file.
    async fn classify(&self, run: &Run, attempt: &mut Attempt) -> Option<(String, String, Unit)> {
        let (sampled, weighed) = attempt.sampled.take()?;
        let unit = self
            .choices
            .lock()
            .await
            .classify(&sampled, weighed, wall_ms(run))
            .clone();
        Some((sampled.label, sampled.domain_key, unit))
    }
}

/// What each attempt is given: a Session drawn from the crawl's Session
/// pool under the Run's settings now, the tier that fetches its request's
/// document, and what every attempt of the Run hands over alike. The pool
/// is out of `pool` only while a write that leases work items draws from
/// it.
struct Pooled<'a> {
    pool: &'a mut Option<SessionPool>,
    settings: PoolSettings,
    fetching: &'a Fetching<'a>,
    handing: &'a std::sync::Arc<RunHandOver>,
}

impl<'a> Pooled<'a> {
    /// Draws from `pool` under `settings`, fetching on `fetching`, each
    /// attempt handing over `handing` with its own part.
    fn of(
        pool: &'a mut Option<SessionPool>,
        settings: PoolSettings,
        (fetching, handing): (&'a Fetching<'a>, &'a std::sync::Arc<RunHandOver>),
    ) -> Self {
        Self {
            pool,
            settings,
            fetching,
            handing,
        }
    }
}

/// How an attempt's tier is chosen, settled as its Session is drawn.
enum Chosen {
    /// On this plan.
    Planned(AttemptPlan),
    /// As a race, unless its Label on its domain, which learned this plan,
    /// is due a sample, which it then takes.
    Racing(AttemptPlan),
}

/// A work item leased, with the Session drawn for its attempt, as that
/// Session runs in the browser, its local addresses, and how its tier is
/// chosen.
struct Drawn {
    leased: Leased,
    session: ServingSession,
    in_browser: PooledSession,
    local: Local,
    chosen: Chosen,
}

/// What a write that leases work items draws their Sessions and tier
/// choices with, on the writer's connection.
struct Drawing {
    labels: std::sync::Arc<labels::Declared>,
    settings: PoolSettings,
    run_tier: SetTier,
    races: bool,
    /// The Run's wall clock as the write was queued, and in milliseconds
    /// since the Unix epoch.
    wall: (SystemTime, u64),
}

impl Drawing {
    /// Draws the Session of `pool` that serves `leased`, and chooses how its
    /// tier is chosen, as its Label and the Run's `fetch.tier` choose and,
    /// under `auto`, on the browser for a domain a Challenge moved there
    /// (ADR 0021), else, when the Run races its tiers, as a sample while its
    /// Label on its domain is due one, else as a race, else as its Label on
    /// its domain learned in `choices`; answers too the choice whose count
    /// of requests served moved, which the write records, so a resumed
    /// crawl goes on counting toward its refresh.
    fn draw(
        &self,
        (pool, choices): (&mut SessionPool, &mut TierChoices),
        leased: Leased,
    ) -> (Drawn, Option<Unit>) {
        let request = &leased.crawl_request;
        let key = self.labels.pools.pool(&request.label);
        let session = pool.draw(&key, &leased.waiting.key, &self.settings, self.wall.0);
        let in_browser = PooledSession {
            id: session.id.clone(),
            source: self.labels.pools.source(&key),
            address_ordinal: pool.address_ordinal(&session.id),
        };
        let to_browser = pool.browser(&leased.waiting.key);
        let local = pool.local_addresses(&session.id);
        // A sitemap's fetch runs no handler and takes no tier choice: its
        // attempt fetches it on the Run's HTTP tier.
        let choosing = if leased.sitemap {
            Choosing::Alone
        } else {
            self.labels.settings.choose(&request.label, self.run_tier)
        };
        // A request keeping its URL's fragment is served only by a tier
        // that runs the page's scripts, which the engine's choice is then
        // (ADR 0120); the start refused it on any forced tier but those.
        let keeps_fragment = !leased.sitemap && self.labels.fragments.keeps(&request.label);
        let browser = AttemptPlan::On(TierChoice {
            tier: FetchTier::Browser,
            forced: None,
        });
        let mut learned = || {
            let (plan, unit) = choices.serve_attempt(
                &request.label,
                &leased.waiting.key,
                &request.url,
                self.wall.1,
            );
            (plan, unit.cloned())
        };
        let (chosen, counted) = match choosing {
            Choosing::Alone | Choosing::Engine if keeps_fragment => {
                (Chosen::Planned(browser), None)
            }
            Choosing::Alone | Choosing::Engine if self.races && !to_browser && !leased.sitemap => {
                let (plan, counted) = learned();
                (Chosen::Racing(plan), counted)
            }
            Choosing::Alone => (Chosen::Planned(AttemptPlan::Alone), None),
            Choosing::Forced(choice) => (Chosen::Planned(AttemptPlan::On(choice)), None),
            Choosing::Engine if to_browser => (Chosen::Planned(browser), None),
            Choosing::Engine => {
                let (plan, counted) = learned();
                (Chosen::Planned(plan), counted)
            }
        };
        let drawn = Drawn {
            leased,
            session,
            in_browser,
            local,
            chosen,
        };
        (drawn, counted)
    }
}

/// The Session pool and tier choices a write that leases work items draws
/// from, sent back to the delivery loop once the write is done with them:
/// when it ran, or when the writer dropped it unrun.
struct Lent {
    held: Option<(SessionPool, TierChoices)>,
    back: Option<oneshot::Sender<(SessionPool, TierChoices)>>,
}

impl Drop for Lent {
    fn drop(&mut self) {
        if let (Some(held), Some(back)) = (self.held.take(), self.back.take()) {
            let _ = back.send(held);
        }
    }
}

/// Admits work items up to each serving Connection's room and the HTTP
/// admissions the engine's Resource governor gives the Run now, leases
/// them and draws the Session of the pool that serves each in one write of
/// run state, committed before any handler request is sent, and starts an
/// attempt for each on that Connection, which holds its admission until it
/// ends (ADR 0040). A Run the governor gave none waits for its turn.
async fn lease_to_servers(
    run: &Run,
    run_leases: &Leases,
    policy: CrawlPolicy,
    mut pooled: Pooled<'_>,
    admission: &mut Admission,
    servers: &mut [Serving],
    attempts: &mut JoinSet<Attempt>,
) -> Result<(), ChargeError> {
    for (place, server) in servers.iter_mut().enumerate() {
        if !server.serves() || server.in_flight >= policy.handler_concurrency {
            continue;
        }
        let mut permits: Vec<Permit> =
            std::iter::from_fn(|| run.governor().try_admit(WorkKind::Http))
                .take(policy.handler_concurrency - server.in_flight)
                .collect();
        if permits.is_empty() {
            return Ok(());
        }
        let served = admit(run, admission, permits.len(), &mut pooled).await?;
        let pool = pooled
            .pool
            .as_mut()
            .ok_or(ChargeError::State(RunStateError::Closed))?;
        for rotated in pool.rotated_within_block() {
            warn_rotated(run, rotated).await;
        }
        pooled.fetching.retire(run, pool, &server.context).await;
        for drawn in served {
            let Drawn {
                leased,
                session,
                in_browser,
                local,
                chosen,
            } = drawn;
            let lease = run_leases.grant(
                &leased.crawl_request.id,
                leased.lease_epoch,
                policy.lease_length(),
            );
            server.in_flight += 1;
            let copies = admission.second_copies();
            let fetching = &pooled.fetching;
            let tiering = fetching.served(&leased, (chosen, local), &copies).await;
            let admission = FetchAdmission::holding(
                permits
                    .pop()
                    .expect("invariant: one admission per work item admitted"),
            );
            let hand_over = fetching.hand_over(
                pooled.handing,
                (&leased, &lease),
                &copies,
                admission.clone(),
            );
            let fetches = tiering.tier().or(leased.sitemap.then_some(FetchTier::Http));
            pooled.fetching.in_flight.lock().await.began(
                run,
                InFlightRequest {
                    request_id: leased.crawl_request.id.clone(),
                    url: leased.crawl_request.url.clone(),
                    phase: if fetches.is_some() {
                        InFlightPhase::Fetching
                    } else {
                        InFlightPhase::Handling
                    },
                    tier: fetches.map(|tier| tier.as_str().to_owned()),
                    attempt: leased.attempt,
                    started_at: run.point_in_time(),
                },
            );
            attempts.spawn(holding(
                admission,
                attempt(
                    run.clone(),
                    place,
                    server.context.clone(),
                    (leased, (session, in_browser), leased_count(run)),
                    lease,
                    (tiering, (hand_over, fetching.code_free.clone())),
                    std::sync::Arc::clone(&pooled.fetching.in_flight),
                ),
            ));
        }
    }
    Ok(())
}

/// The Session pool `pool` holds: back in it once each write that drew
/// from it answered.
fn held(pool: &mut Option<SessionPool>) -> &mut SessionPool {
    pool.as_mut()
        .expect("invariant: the Session pool is back once each write that drew from it answered")
}

/// Runs `attempt`, holding its HTTP `admission` until it ends, unless its
/// document's fetch gave it back first.
async fn holding(admission: FetchAdmission, attempt: impl Future<Output = Attempt>) -> Attempt {
    let ended = attempt.await;
    admission.give_back();
    ended
}

/// What a crawl with nothing to deliver now does next.
enum Waited {
    /// It ended the Run, or the Run stopped at once.
    Ended,
    /// The Run is asked to stop at its next safe point.
    Stop(StopReason),
    /// `run.finish` let it finish, a Connection joined to serve it, or a
    /// seed was added: look for work once more.
    Again,
}

/// Finishes a crawl with no work item left, with what its bounds and caps
/// cut as its Completeness reasons, unless `crawl.keep_alive`, one of the
/// settings the Run resolved as it opened, holds it until
/// `run.finish` lets it finish: until then it waits for new work, as the
/// Run's own timeout and a stop still end it (RT-12). Before it finishes,
/// it closes its intake in a job of the Run state's writer that looks once
/// more for a pending work item: a re-admission or a seed added since its
/// last look is delivered, and one coming later is refused (#914, #1005).
async fn drained(
    run: &Run,
    resolved: &Resolved,
    admission: &mut Admission,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
) -> Waited {
    let keeps_alive: bool = resolved.get(&CRAWL_KEEP_ALIVE);
    if !keeps_alive || run.is_let_finish() {
        let intake = arrivals.intake.clone();
        // Its answer promises nothing outside (#2117): the Run's end, which
        // follows it, syncs, and so does a later call's refusal, as its own
        // write commits after the close.
        match run
            .state()
            .write_deferred(move |connection| intake.close_if_drained(connection))
            .await
        {
            Ok(true) => {}
            Ok(false) => return Waited::Again,
            Err(error) => {
                tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                end(run, admission, failed(run, &error)).await;
                return Waited::Ended;
            }
        }
        let ending = match run.state().read(completeness::short_of_scope).await {
            Ok(reasons) => Ending::Finished(reasons),
            Err(error) => {
                tracing::error!(%error, run_id = %run.id(), "what cut the crawl could not be read");
                failed(run, &error)
            }
        };
        end(run, admission, ending).await;
        return Waited::Ended;
    }
    progress::show_waits(run, &[], resolved, run.clock().now(), Room::None);
    progress::show_idle(run, true);
    waited(run, admission, servers, arrivals, true).await
}

/// Waits, with nothing to deliver now, until the Run stops at once, is
/// asked to stop at its next safe point, a Connection joins to serve it, a
/// seed is added or, `until_finish`, `run.finish` lets it finish.
async fn waited(
    run: &Run,
    admission: &mut Admission,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    until_finish: bool,
) -> Waited {
    tokio::select! {
        biased;
        () = run.cancellation().cancelled() => {
            admission.close().await;
            release(run).await;
            Waited::Ended
        }
        reason = run.stopping() => Waited::Stop(reason),
        () = run.let_finish(), if until_finish => Waited::Again,
        Some(server) = arrivals.joined.recv() => {
            servers.push(Serving::joined(server));
            Waited::Again
        }
        () = arrivals.seeded.notified() => Waited::Again,
    }
}

/// What a crawl with no attempt in flight does next.
enum Idle {
    /// Admit the work items its queues now hold.
    Admit,
    /// Wait until its admission is ready or the Run stops.
    Wait,
    /// Finish: no work item is pending.
    Done,
}

/// What a crawl with no attempt in flight does next: its queues filled
/// again from the pending work items when they hold none ready.
async fn when_idle(run: &Run, admission: &mut Admission) -> Result<Idle, RunStateError> {
    if admission.ready_at(run.clock().now()).is_some() {
        return Ok(Idle::Wait);
    }
    key_new(run, admission).await?;
    fill(run, admission, None).await?;
    if admission.ready_at(run.clock().now()).is_some() {
        return Ok(Idle::Admit);
    }
    Ok(if run.state().read(work_items::any_pending).await? {
        Idle::Wait
    } else {
        Idle::Done
    })
}

/// What settling one ended attempt keeps of it once its Session is scored,
/// for its domain's turn and queue after its commit.
struct Settling {
    waiting: Waiting,
    merged: Vec<(String, String)>,
    domain: Ended,
    held: Option<SystemTime>,
}

/// What one ended attempt's outcome writes beside its work item's end:
/// what its Session's score changed, and its tier choice once its sample
/// was classified.
struct Scored {
    changes: Changes,
    chosen: Option<(String, String, Unit)>,
}

/// Settles the attempts that ended, `ended`, in the order they ended,
/// under the Run's policy and pool settings, resolved as it opened (ADR
/// 0138): takes each out of the requests in flight
/// its progress shows, frees its place on its Connection and scores its
/// Session; queues each one's outcome as one job of the run state's
/// writer, in their order, so the writer takes them in one group rather
/// than the loop waiting for each in turn (#1765), and each one's failure
/// rolls back its own alone (ADR 0138); then returns each one's domain
/// turn and puts each retried work item back in its queue, in their order,
/// offers its domain's queue each work item an outcome added, keyed as it
/// was added, and journals what the queues changed.
async fn settled(
    run: &Run,
    (resolved, read): (&Resolved, Read),
    admission: &mut Admission,
    (pool, fetching): (&mut SessionPool, &Fetching<'_>),
    servers: &mut [Serving],
    ended: Vec<Attempt>,
) {
    let (attempts, mut settlings, scores) =
        scored(run, read, (&mut *pool, fetching), servers, ended).await;
    for attempt in &attempts {
        fetching
            .retire(run, pool, &servers[attempt.server].context)
            .await;
    }
    let mut queued = Vec::with_capacity(attempts.len());
    for ((attempt, settling), scored) in attempts.into_iter().zip(&settlings).zip(scores) {
        let held = settling.held.is_some();
        queued.push(queue_outcome(run, (resolved, read.policy), held, attempt, scored).await);
    }
    let mut retried = Vec::with_capacity(queued.len());
    let mut keyed = Vec::new();
    for (outcome, settling) in join_all(queued.into_iter().map(QueuedOutcome::finished))
        .await
        .into_iter()
        .zip(&mut settlings)
    {
        settling.merged.extend(outcome.merged);
        keyed.extend(outcome.keyed);
        retried.push(outcome.retried);
    }
    returned(run, admission, settlings.into_iter().zip(retried)).await;
    for waiting in keyed {
        admission.enqueue(waiting);
    }
    if let Err(error) = key_new(run, admission).await {
        tracing::warn!(%error, run_id = %run.id(), "the requests a handler enqueued were not keyed yet");
    }
    journal(run, admission).await;
}

/// Takes each attempt of `ended` out of the requests in flight its
/// progress shows, frees its place on its Connection and scores its
/// Session in `pool` under what the loop `read`, in their order, answering them with
/// what each one's settling keeps and what its outcome writes of its
/// score and tier choice.
async fn scored(
    run: &Run,
    read: Read,
    (pool, fetching): (&mut SessionPool, &Fetching<'_>),
    servers: &mut [Serving],
    ended: Vec<Attempt>,
) -> (Vec<Attempt>, Vec<Settling>, Vec<Scored>) {
    let mut attempts = Vec::with_capacity(ended.len());
    let mut settlings = Vec::with_capacity(ended.len());
    let mut scores = Vec::with_capacity(ended.len());
    for mut attempt in ended {
        let chosen = fetching.classify(run, &mut attempt).await;
        fetching.time(&mut attempt).await;
        fetching
            .in_flight
            .lock()
            .await
            .ended(run, &attempt.leased.crawl_request.id);
        let server = &mut servers[attempt.server];
        server.in_flight -= 1;
        if let Answer::ClientGone = attempt.answer {
            server.gone = true;
        }
        let waiting = attempt.leased.waiting.clone();
        let merged = std::mem::take(&mut attempt.taught.merged);
        let domain = domain_ending(
            &attempt.answer,
            attempt.document.as_ref(),
            read.proxied,
            run.clock().now(),
            run.clock().wall(),
        );
        if attempt.moved {
            pool.moved_to_browser(&waiting.key);
        }
        pool.settle(
            &attempt.session,
            &waiting.key,
            effect(&attempt.answer),
            &read.pool,
        );
        let held = held_past_bound(
            &attempt.answer,
            attempt.handed_at.unwrap_or_else(|| run.clock().wall()),
            read.longest_wait,
        );
        settlings.push(Settling {
            waiting,
            merged,
            domain,
            held,
        });
        scores.push(Scored {
            changes: pool.changes(),
            chosen,
        });
        attempts.push(attempt);
    }
    (attempts, settlings, scores)
}

/// Returns, in their order, each settled attempt's domain turn to
/// `admission`, holding its domain first until the time its response asked
/// past the Run's longest wait, puts its work item back in its queue when
/// it was `retried`, takes out of their queues the requests a rule it
/// taught merged, and fills those queues again.
async fn returned(
    run: &Run,
    admission: &mut Admission,
    settled: impl Iterator<Item = (Settling, bool)>,
) {
    let mut keys = BTreeSet::new();
    for (settling, retried) in settled {
        let Settling {
            waiting,
            merged,
            domain,
            held,
        } = settling;
        if let Some(asked) = held {
            let clock = run.clock();
            if let Ok(wait) = asked.duration_since(clock.wall()) {
                admission.refuse(&waiting.key, clock.now() + wait, asked);
            }
        }
        admission
            .ended(&waiting.id, &waiting.key, run.clock().now(), domain)
            .await;
        if retried {
            count_retry(run);
            admission.enqueue(waiting);
        }
        for (id, key) in merged {
            admission.withdraw(&key, &id);
            keys.insert(key);
        }
    }
    for key in keys {
        if let Err(error) = fill(run, admission, Some(key)).await {
            tracing::warn!(%error, run_id = %run.id(), "a queue a learned rule thinned was not filled again");
        }
    }
}

/// Opens the Run's admission and Session pool, or ends the Run as failed
/// when they could not open.
async fn opened(run: &Run, settings: &RunSettings) -> Option<(Admission, Option<SessionPool>)> {
    match open(run, settings).await {
        Ok((admission, pool)) => Some((admission, Some(pool))),
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's admission could not open");
            run.end(failed(run, &error)).await;
            None
        }
    }
}

/// Opens the Run's admission, the gate Foxhound's composition passes in,
/// keys the work items not keyed yet and fills each domain's queue; and
/// opens its Session pool from its run state, on an `Entropy` of its own.
async fn open(run: &Run, settings: &RunSettings) -> Result<(Admission, SessionPool), String> {
    let resolved = settings.resolved();
    let mut admission = run
        .admission(resolved)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("the composition passes in no gate in admission")?;
    key_new(run, &mut admission)
        .await
        .map_err(|error| error.to_string())?;
    fill(run, &mut admission, None)
        .await
        .map_err(|error| error.to_string())?;
    let entropy = run.fork_entropy().await;
    let pool = run
        .state()
        .read(move |connection| SessionPool::open(connection, entropy))
        .await
        .map_err(|error| error.to_string())?;
    let pool = match Rotation::of(resolved) {
        Some(rotation) => pool.with_local_addresses(LocalAddressing::new(
            rotation,
            BlockSizes::of(resolved),
            std::sync::Arc::new(System),
            // No Session's every request leaves through the Run's proxy
            // yet: the HTTP tiers send direct from its addresses, so a block
            // always blocks the address the site saw (ADR 0044's #682 note).
            false,
        )),
        None => pool,
    };
    Ok((admission, pool))
}

/// The local addresses a Session drawn from the pool holds: `None`
/// without `egress.local_addresses` (ADR 0044).
type Local = Option<Result<SessionAddresses, Unavailable>>;

/// The interface `egress.local_addresses` names, if it names one.
fn local_interface(resolved: &Resolved) -> Option<String> {
    match LocalAddresses::parse(resolved.value(&EGRESS_LOCAL_ADDRESSES)) {
        Ok(LocalAddresses::Interface(name)) => Some(name),
        _ => None,
    }
}

/// What the Run's own fetches, robots.txt's and sitemaps', send from under
/// `egress.local_addresses`: the addresses of the rotation's first
/// position, which the Run's first Session takes (ADR 0044).
///
/// # Errors
///
/// A message when the host no longer holds them.
fn run_source(
    resolved: &Resolved,
    interface: Option<&String>,
) -> Result<Option<LocalSource>, String> {
    let Some(rotation) = Rotation::of(resolved) else {
        return Ok(None);
    };
    let addresses = rotation
        .session(&System, 0)
        .map_err(|unavailable| goodfetch_engine::CallError::from(unavailable).to_string())?;
    Ok(Some(LocalSource::new(addresses, interface.cloned())))
}

/// Whether the Run sets a proxy, which forwards an `http:` URL itself on
/// the routes that take it (ADR 0053).
fn proxied(resolved: &Resolved) -> bool {
    [&PROXY_SERVER, &PROXY_PROVIDER]
        .into_iter()
        .any(|setting| !resolved.value(setting).is_null())
}

/// Hands the Run's report the warning that `rotated` took an address within
/// a block blocked on its domain (ADR 0044).
async fn warn_rotated(run: &Run, rotated: Rotated) {
    let warning = RunWarning {
        code: LOCAL_ADDRESS_ROTATED_WITHIN_BLOCK.to_owned(),
        message: format!(
            "Every block of `egress.local_addresses` was blocked on {} during the run, so the Session that replaced a blocked one took an address within a blocked block.",
            rotated.domain_key
        ),
        sessions: vec![rotated.session.0],
        folder: None,
    };
    if let Err(error) = run.warn(warning).await {
        tracing::warn!(%error, run_id = %run.id(), "the warning of a Session rotating within a block was not kept");
    }
}

/// How many work items not keyed yet [`key_new`] reads at a time, so a
/// stream of added seeds never sits in memory whole.
const KEYED_AT_ONCE: usize = 1024;

/// Keys each work item not keyed yet, the seeds at the start, those a
/// handler enqueued since and those added to the running crawl, and offers
/// it to its domain's queue, which takes it unless full; one it refuses
/// waits in run state until its queue fills again.
async fn key_new(run: &Run, admission: &mut Admission) -> Result<(), RunStateError> {
    loop {
        let unkeyed = run
            .state()
            .read(|connection| work_items::unkeyed(connection, KEYED_AT_ONCE))
            .await?;
        let read = unkeyed.len();
        key(run, admission, unkeyed).await?;
        if read < KEYED_AT_ONCE {
            return Ok(());
        }
    }
}

/// Keys each of `unkeyed`, as [`key_new`] does.
async fn key(
    run: &Run,
    admission: &mut Admission,
    unkeyed: Vec<(String, Waiting)>,
) -> Result<(), RunStateError> {
    if unkeyed.is_empty() {
        return Ok(());
    }
    let keyed: Vec<Waiting> = unkeyed
        .into_iter()
        .map(|(url, waiting)| Waiting {
            key: admission.key(&url),
            ..waiting
        })
        .collect();
    let keys: Vec<(String, String)> = keyed
        .iter()
        .map(|waiting| (waiting.id.clone(), waiting.key.clone()))
        .collect();
    // Keys promise nothing outside (#2117): the outcome of the attempt that
    // claims the item syncs them, and a power cut that takes them leaves
    // the items unkeyed, which the next look keys again.
    run.state()
        .write_deferred(move |connection| work_items::set_keys(connection, &keys))
        .await?;
    for waiting in keyed {
        admission.enqueue(waiting);
    }
    Ok(())
}

/// Fills the queue of `key`, or of every key when `None`, from the pending
/// work items, at most its bound.
async fn fill(
    run: &Run,
    admission: &mut Admission,
    key: Option<String>,
) -> Result<(), RunStateError> {
    let bound = admission.bound();
    let waiting = run
        .state()
        .read(move |connection| work_items::waiting(connection, key.as_deref(), bound))
        .await?;
    for waiting in waiting {
        admission.enqueue(waiting);
    }
    Ok(())
}

/// Admits up to `room` work items ready now, leases them and draws their
/// Sessions, then fills again each domain's queue that drained. Each one
/// first charges the request it is against the Limits that hold the Run
/// (ADR 0041): once one has nothing left, or one the engine cannot
/// evaluate covers it, the Run is asked to stop at its next safe point and
/// nothing more is admitted.
async fn admit(
    run: &Run,
    admission: &mut Admission,
    room: usize,
    pooled: &mut Pooled<'_>,
) -> Result<Vec<Drawn>, ChargeError> {
    let now = run.clock().now();
    let mut ids = Vec::new();
    let mut keys = Vec::new();
    let mut drained = Vec::new();
    let mut refused = Vec::new();
    let mut charges = UnjournaledCharges::default();
    while ids.len() < room {
        let Some(admitted) = admission.admit(now).await else {
            break;
        };
        if let Some(until) = admitted.refused_until {
            if admitted.drained {
                drained.push(admitted.key.clone());
            }
            refused.push((admitted, until));
            continue;
        }
        match run.charge_unjournaled(&mut charges, REQUESTS, 1).await {
            Ok(()) => {}
            Err(ChargeError::Refused(Refusal::Reached(_) | Refusal::NotUnderstood(_))) => break,
            Err(error) => return Err(error),
        }
        if admitted.drained {
            drained.push(admitted.key.clone());
        }
        ids.push(admitted.id);
        keys.push(admitted.key);
    }
    if !refused.is_empty() {
        end_unsent(run, admission, refused).await?;
    }
    let asked = ids.clone();
    let drawn = if ids.is_empty() {
        Vec::new()
    } else {
        leased_and_drawn(run, charges, ids, pooled).await?
    };
    // A work item a rule learned later merged after the gate took it is
    // no longer pending: it gives its domain's turn back unsent.
    for (id, key) in asked.iter().zip(&keys) {
        if !drawn
            .iter()
            .any(|drawn| drawn.leased.crawl_request.id == *id)
        {
            admission.ended(id, key, now, Ended::Done).await;
        }
    }
    for key in drained {
        fill(run, admission, Some(key)).await?;
    }
    Ok(drawn)
}

/// Leases each pending work item of `ids` and draws the Session of
/// `pooled`'s pool that serves each, in one write of run state that also
/// journals `charges` and holds what the pool's draws changed and each
/// tier choice's count of requests served: each attempt's claim, one job
/// of the writer (ADR 0138). The pool and the tier choices are the write's
/// while it runs.
async fn leased_and_drawn(
    run: &Run,
    unjournaled: UnjournaledCharges,
    ids: Vec<String>,
    pooled: &mut Pooled<'_>,
) -> Result<Vec<Drawn>, ChargeError> {
    let pool = pooled
        .pool
        .take()
        .ok_or(ChargeError::State(RunStateError::Closed))?;
    let fetching = pooled.fetching;
    let mut choices = fetching.choices.lock().await;
    let (back, returned) = oneshot::channel();
    let mut lent = Lent {
        held: Some((pool, std::mem::take(&mut *choices))),
        back: Some(back),
    };
    let drawing = Drawing {
        labels: std::sync::Arc::clone(&fetching.labels),
        settings: pooled.settings,
        run_tier: fetching.run_tier,
        races: fetching.races,
        wall: (run.clock().wall(), wall_ms(run)),
    };
    let written = run
        .write_charged(unjournaled, move |connection| {
            let (pool, choices) = lent
                .held
                .as_mut()
                .expect("invariant: a write that draws holds the pool until it is dropped");
            let mut drawn = Vec::new();
            let mut counted = Vec::new();
            for leased in work_items::lease(connection, &ids)? {
                let key = (
                    leased.crawl_request.label.clone(),
                    leased.waiting.key.clone(),
                );
                let (one, unit) = drawing.draw((pool, choices), leased);
                drawn.push(one);
                counted.extend(unit.map(|unit| (key, unit)));
            }
            let changes = pool.changes();
            if !changes.is_empty() {
                changes.write(connection)?;
            }
            for ((label, domain_key), unit) in &counted {
                tier_choice::record(connection, label, domain_key, unit)?;
            }
            Ok(drawn)
        })
        .await;
    // The write's job is dropped once it ran, or unrun once the writer
    // stopped: either way the pool and the choices come back.
    if let Ok((pool, taken)) = returned.await {
        *pooled.pool = Some(pool);
        *choices = taken;
    }
    written
}

/// Writes what the gate changed into the run file.
async fn journal(run: &Run, admission: &mut Admission) {
    let clock = run.clock();
    if let Some(journal) = admission.journal(clock.wall(), clock.now())
        && let Err(error) = run
            .state()
            // A domain's hold or backoff joins the next group that syncs
            // (ADR 0138's note of #1926).
            .write_deferred(move |connection| journal(connection))
            .await
    {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's per-domain queues were not journaled");
    }
}

/// Sleeps until `moment`, if any.
async fn sleep_until(clock: Clock, moment: Option<Instant>) {
    if let Some(moment) = moment {
        clock.sleep_until(moment).await;
    }
}

/// One more retry in `goodfetch.work_item.retried`.
fn count_retry(run: &Run) {
    run.metrics()
        .meter()
        .u64_counter(WORK_ITEM_RETRIED.name)
        .with_description(WORK_ITEM_RETRIED.description)
        .with_unit(WORK_ITEM_RETRIED.unit)
        .build()
        .add(1, &[]);
}

/// An ended attempt's outcome, queued as one job of the run state's
/// writer, and what finishing it needs once that job answers.
struct QueuedOutcome {
    leased: Leased,
    lease: WorkItemLease,
    run: Run,
    /// Whether it commits its answer.
    commits: bool,
    written: Pin<Box<dyn Future<Output = Written> + Send>>,
}

/// What an outcome's job answered.
type Written = Result<(attempt_outcome::Ended, Vec<(String, String)>), RunStateError>;

/// What an attempt's outcome left the delivery to do once it was written:
/// the pending requests a rule its page taught merged, by ID with their
/// domain key, each work item it added keyed, in order, and whether its
/// work item waits again for a retry.
struct Finished {
    merged: Vec<(String, String)>,
    keyed: Vec<Waiting>,
    retried: bool,
}

/// Queues `attempt`'s outcome as one job of the run state's writer (ADR
/// 0138), with what its Session's score changed and its tier choice,
/// `scored`, and what its page teaches the Frontier when it left that to
/// its outcome: its Records committed once while its lease holds, its
/// links admitted and keyed, or its handler's skip, its block, or its work
/// item retried or failed, under the Run's `policy`, with no retry when its
/// failure's site asked for a time further away than the Run waits
/// (`held`), or its sitemap's entries admitted under the Run's domain
/// policy as `resolved` holds it, within the scope the run file holds,
/// the fetch ended with no outcome (ADR 0093).
async fn queue_outcome(
    run: &Run,
    (resolved, policy): (&Resolved, CrawlPolicy),
    held: bool,
    attempt: Attempt,
    scored: Scored,
) -> QueuedOutcome {
    let Attempt {
        leased,
        lease,
        answer,
        taught,
        ..
    } = attempt;
    let (work_item, lease_epoch) = (leased.crawl_request.id.clone(), leased.lease_epoch);
    let masks = run.secrets().mask().masks().clone();
    let tries = leased.tries;
    let failure = move |failure, retryable| retried_or_failed(policy, tries, failure, retryable);
    let mut commits = false;
    let unsuccessful = match answer {
        Answer::Handled(CrawlRequestHandled {
            skipped: Some(skip),
            ..
        }) => Unsuccessful::Skipped {
            reason: SkipReason::Handler,
            message: Some(skip.message),
        },
        Answer::Refused(reason) => refused(reason),
        Answer::RobotsTxtUnreachable(message) if policy.may_retry(leased.tries) => {
            Unsuccessful::Retried(Failure::new(SITE_PREFERENCES_REFUSED.name, &message))
        }
        Answer::RobotsTxtUnreachable(_) => refused(SkipReason::RobotsTxt),
        Answer::SitemapRead(document) => {
            let work = run.metrics().counted_work().clone();
            work.count(Row::SitemapTransactions);
            let ending = attempt_outcome::Ending::Sitemap {
                work_item,
                lease_epoch,
                egress: resumed_domain_policy(resolved, run.id()),
                document,
                work,
            };
            return queued(run, (leased, lease), (scored, taught, ending), false).await;
        }
        Answer::Handled(handled) => {
            match lease.committing(run.state(), &masks, handled.records, run.point_in_time()) {
                Ok(commit) => {
                    commits = true;
                    let refused: Refusing =
                        Box::new(move |refusal: goodfetch_engine::CallError| {
                            failure(
                                kept_failure(
                                    &masks,
                                    refusal.code().to_owned(),
                                    refusal.message(),
                                    refusal.details().cloned().unwrap_or_default(),
                                ),
                                refusal.retryable(),
                            )
                        });
                    let ending = attempt_outcome::Ending::Commit { commit, refused };
                    return queued(run, (leased, lease), (scored, taught, ending), commits).await;
                }
                Err(_) => failure(
                    Failure::new(LEASE_LOST.name, LEASE_LOST.description),
                    LEASE_LOST.retryable,
                ),
            }
        }
        Answer::ClientGone => failure(
            Failure::new(CLIENT_GONE.name, CLIENT_GONE.description),
            CLIENT_GONE.retryable,
        ),
        Answer::LeaseEnded => failure(
            Failure::new(HANDLER_LOST.name, HANDLER_LOST.description),
            HANDLER_LOST.retryable,
        ),
        Answer::Failed {
            code,
            retryable,
            message,
            details,
        } => {
            let failed = kept_failure(&masks, code, &message, details);
            match block_cause(&failed.code, &failed.details) {
                Some(cause) => Unsuccessful::Blocked(cause),
                None if held => Unsuccessful::Failed(failed.with_fix(RETRY_AFTER_FIX)),
                None => failure(failed, retryable),
            }
        }
    };
    let ending = attempt_outcome::Ending::End {
        work_item,
        lease_epoch,
        unsuccessful,
        ended_at: run.point_in_time(),
    };
    queued(run, (leased, lease), (scored, taught, ending), commits).await
}

/// Queues the outcome of `leased`'s attempt under `lease`, ending as
/// `ending` says beside `scored` and what `taught` leaves to it, in the
/// run state's writer's queue now.
async fn queued(
    run: &Run,
    (leased, lease): (Leased, WorkItemLease),
    (scored, taught, ending): (Scored, Taught, attempt_outcome::Ending),
    commits: bool,
) -> QueuedOutcome {
    let outcome = Outcome {
        scored: scored.changes,
        chosen: scored.chosen,
        learning: taught.learning.map(|learning| *learning),
        ending,
    };
    let written: Pin<Box<dyn Future<Output = Written> + Send>> = match run.state().reserve().await {
        Ok(place) => Box::pin(place.write(move |connection| outcome.write(connection))),
        Err(error) => Box::pin(std::future::ready(Err(error))),
    };
    QueuedOutcome {
        leased,
        lease,
        run: run.clone(),
        commits,
        written,
    }
}

impl QueuedOutcome {
    /// Waits for its job's answer, then releases its lease and finishes
    /// what the job committed: a committed attempt's suspected trap links'
    /// Provenance and the files it took out, and the failure an attempt
    /// ended with counted in its Run's outcomes.
    async fn finished(self) -> Finished {
        let Self {
            leased,
            lease,
            run,
            commits,
            written,
        } = self;
        let mut finished = Finished {
            merged: Vec::new(),
            keyed: Vec::new(),
            retried: false,
        };
        let ended = match written.await {
            Ok((ended, merged)) => {
                finished.merged = merged;
                ended
            }
            Err(error) => {
                if commits {
                    // Traced as a commit the run file refused, its lease
                    // left to end.
                    if let Err(CommitError::RunState(error)) = lease.committed(Err(error)) {
                        tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                    }
                } else {
                    lease.release();
                    tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                }
                return finished;
            }
        };
        match ended {
            attempt_outcome::Ended::Committed(committed) => {
                let committed = lease
                    .committed(Ok(Some(Ok(committed))))
                    .expect("invariant: a commit written is committed");
                lease.release();
                crate::enqueue::links::mark_suspected(
                    &run,
                    &leased.crawl_request.id,
                    &committed.suspected,
                )
                .await;
                goodfetch_engine::key_value_store::forget_files(&run, committed.files).await;
                finished.keyed = committed.keyed;
            }
            attempt_outcome::Ended::LeaseLost => {
                let _ = lease.committed(Ok(None));
                lease.release();
            }
            attempt_outcome::Ended::Unsuccessful {
                ended,
                recorded,
                retried,
            } => {
                lease.release();
                if ended {
                    if let Some(code) = recorded {
                        run.record_outcome(&code);
                    }
                    finished.retried = retried;
                } else {
                    tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "an attempt past its lease commits nothing");
                }
            }
            attempt_outcome::Ended::SitemapRead(read) => {
                lease.release();
                if !read {
                    tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "a sitemap's fetch past its lease reads nothing");
                }
            }
        }
        finished
    }
}

/// A handler's failure as the Run's state keeps it: no value of `masks`,
/// those the Run resolved, stays in its message or details (spec #564).
fn kept_failure(
    masks: &goodfetch_engine::mask::Masks,
    code: String,
    message: &str,
    details: Map<String, Value>,
) -> Failure {
    let mut kept = Value::Object(details);
    masks.apply_value(&mut kept, Scope::Kept);
    let Value::Object(details) = kept else {
        unreachable!("invariant: masking an object leaves an object")
    };
    Failure {
        code,
        message: masks.apply(message, Scope::Kept).into_owned(),
        details,
    }
}

/// A request the site's preferences refused before its handler ran,
/// skipped with `reason` (ADR 0086).
fn refused(reason: SkipReason) -> Unsuccessful {
    Unsuccessful::Skipped {
        reason,
        message: None,
    }
}

/// How `failure` on a work item's `attempt`th attempt since its latest
/// admission ends it under
/// `policy`: retried while it is `retryable` and retries are left, failed
/// otherwise; a handler's own failure is retryable only as
/// `crawl.retry.handler_failed` says.
fn retried_or_failed(
    policy: CrawlPolicy,
    attempt: u32,
    failure: Failure,
    retryable: bool,
) -> Unsuccessful {
    let retryable = if failure.code == HANDLER_FAILED.name {
        policy.retry_handler_failed
    } else {
        retryable
    };
    if retryable && policy.may_retry(attempt) {
        Unsuccessful::Retried(failure)
    } else {
        Unsuccessful::Failed(failure)
    }
}

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run) {
    if let Err(error) = run.state().write(work_items::release_leases).await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
