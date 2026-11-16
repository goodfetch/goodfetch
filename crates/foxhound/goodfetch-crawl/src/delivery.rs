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
//! (`run.add_seed`, `run.add_seeds`) is admitted by it, idle or not, as
//! every other task's question of the crawl's Run state is answered by it
//! (ADR 0142; `run_state`), and queued under its domain as it is written. When the Run is asked to stop at its next safe point,
//! such as once its `timeout.run` passed, it admits nothing new, lets each
//! attempt in flight commit or be abandoned under its lease, and then stops
//! the Run. When the Run stops at once, by an abort, the attempts in flight
//! are abandoned and their work items return to pending without counting;
//! when
//! no Connection serves the crawl any more, the driver delivers nothing
//! more and waits for the Run to stop or a Connection to join. Each attempt
//! counts in `goodfetch.work_item.leased` until it ends, and each retry in
//! `goodfetch.work_item.retried`, on the meter the Run hands out (ADR 0050).
//! A pending work item a rule the Frontier learned later, at the end of a
//! batch the run state's writer commits (#2169), merged leaves its
//! domain's queue, and one the gate already took gives its turn back
//! unsent; one that rule's stop made pending again joins its domain's
//! queue as that queue is filled again. Each attempt runs on the tier the crawl's tier choice plans for
//! its Label on its domain (`tier_choice`); a sample's outcome is
//! classified, and the choice written in the run file, as its attempt
//! settles, before the next admission.
//!
//! Size: one delivery loop, the admission, leases, attempts and settling it
//! drives, kept whole so every work item's way from pending to its ending
//! passes through one loop.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use futures_util::StreamExt as _;
use futures_util::future::join_all;
use futures_util::stream::FuturesUnordered;
use goodfetch_dataset::Datasets;
use goodfetch_engine::CallContext;
use goodfetch_engine::admission::{Admission, Ended, SecondCopies, Waiting, Wakes};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::ledger::Refusal;
use goodfetch_engine::local_addresses::{BlockSizes, LocalAddresses, Rotation, System};
use goodfetch_engine::mask::Scope;
use goodfetch_engine::metrics::Level;
use goodfetch_engine::runs::{ChargeError, Ending, Run, UnjournaledCharges};
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_fetch_tier::{
    ContentTypes, FetchAdmission, FetchTiers, HandOver, HopChecks, HttpFrom, HttpTier, LocalSource,
    RunHandOver, SetTier, TierChoice, UntouchedDemand, UntouchedReaders,
};
use goodfetch_frontier::Bounds;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::egress::EGRESS_LOCAL_ADDRESSES;
use goodfetch_protocol::foxhound::crawl::{
    CLIENT_GONE, CRAWL_CONTENT_TYPES, CRAWL_KEEP_ALIVE, CrawlRequestHandled, HANDLER_LOST,
    LEASE_LOST, WORK_ITEM_QUEUED, WORK_ITEM_RETRIED,
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
use goodfetch_protocol::foxhound::settings::{
    DATASET_RECORDS, FETCH_BLOCK_CLASSIFICATION, PROVENANCE_LOG,
};
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::proxy::{PROXY_PROVIDER, PROXY_SERVER};
use goodfetch_protocol::page::{SITE_PREFERENCES_REFUSED, SessionId};
use goodfetch_protocol::run::{
    EndingError, NothingDone, RUN_STATE_UNAVAILABLE, RunDetails, RunWarning, StopReason,
};
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::usage::REQUESTS;
use goodfetch_session_pool::{
    Changes, Effect, LocalAddressing, PoolSettings, Rotated, SendsFrom, SessionPool,
};
use serde_json::{Map, Value};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::attempt_outcome::{self, Outcome, Refusing};
use crate::code_free::{self, CodeFree};
use crate::invariants::Invariants;
use crate::labels;
use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::{CrawlPolicy, resumed_domain_policy};
use crate::progress::{self, InFlight, Room, SharedInFlight};
use crate::run_state::{
    Asking, Chosen, Drawing, Drawn, DrawnSession, Local, Question, QueuedEnd, RunState, StateError,
    Stored, Tally, Unkeyed, Within,
};
use crate::serving::{Crawls, Opened, Server};
use crate::tier_choice::{self, AttemptPlan, Sampling, TierChoices, Unit};
use crate::tier_race::Raced;
use crate::work_items::{self, Changed, Failure, Leased, Unsuccessful};
use goodfetch_protocol::foxhound::progress::{HELD_DOMAINS, InFlightPhase, InFlightRequest};

mod attempt;
mod failure;
mod merged;

use failure::{AttemptFailure, Why};
mod pages;
mod politeness;
mod watch;

use attempt::{
    Answer, Attempt, Escalating, Lessons, Tiering, TransportSession, effect, handler_lease,
    leased_count, run_hand_over,
};
use merged::merged;
use pages::{Pages, Start};
use politeness::{RETRY_AFTER_FIX, domain_ending, end_unsent, held_past_bound};
use watch::Watch;

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
    let run_tier = FetchTiers::run_tier(settings.resolved());
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
        // Every request may take the `http` tier, forced or by the engine's
        // choice: the Run cannot go on.
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's HTTP tier could not be built");
            run.end(failed(&run, &error)).await;
            crawls.close(run.id()).await;
            return;
        }
    };
    let untouched = Untouched::of(&run, &labels, code_free.as_ref());
    let fetching = Fetching {
        tiers: &tiers,
        http: http.as_ref(),
        interface,
        labels: std::sync::Arc::new(labels),
        run_tier,
        http_from: HttpFrom::of_run(settings.resolved()),
        run_encoding: settings
            .resolved()
            .get::<Option<&str>>(&FETCH_ENCODING)
            .map(str::to_owned),
        // Its shape was checked when the crawl started.
        run_content_types: ContentTypes::of(settings.resolved().value(&CRAWL_CONTENT_TYPES))
            .unwrap_or_default(),
        races,
        light: FetchTiers::light_allowed(settings.resolved()),
        in_flight: std::sync::Arc::new(tokio::sync::Mutex::new(InFlight::default())),
        untouched,
        code_free: code_free.map(std::sync::Arc::new),
        asking: arrivals.asking.clone(),
    };
    for (setting, code, message) in SWITCHED_OFF {
        if !settings.resolved().get::<bool>(setting) {
            warn_switched_off(&run, code, message).await;
        }
    }
    arrivals
        .invariants
        .prepare(&run, &work_items::SITEMAP_READ)
        .await;
    // What every attempt hands over alike, built once (ADR 0138), with
    // how its redirect hops ask the loop (ADR 0142).
    let hops: std::sync::Arc<dyn HopChecks> = std::sync::Arc::new(arrivals.asking.clone());
    let handing = run_hand_over(
        &run,
        settings.resolved(),
        fetching.http,
        (&arrivals.egress, hops),
    )
    .await;
    let fetching = (&fetching, &handing);
    let choices = choices.sampled_as(sampling(settings.resolved()));
    deliver(
        &run,
        &mut servers,
        &mut arrivals,
        (&settings, choices),
        fetching,
    )
    .await;
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
/// per-page feature its settings switch off (ADR 0138), in the open batch:
/// the crawl hands it over again each time it opens (ADR 0142).
async fn warn_switched_off(run: &Run, code: &str, message: &str) {
    let warning = RunWarning {
        code: code.to_owned(),
        message: message.to_owned(),
        sessions: Vec::new(),
        folder: None,
    };
    if let Err(error) = run.warn_once_deferred(warning).await {
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
        count: usize::try_from(resolved.get::<u64>(&FETCH_SAMPLE_COUNT)).unwrap_or(built_in.count),
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

/// What the delivery's loop does once no page is open and no settlement
/// is being written.
enum Idled {
    /// The Run ended: the loop returns.
    Ended,
    /// The loop looks for work again, stopping for the reason it holds.
    Again(Option<StopReason>),
    /// The loop waits for its next event.
    Waits,
}

/// The delivery loop's turn once nothing is in flight: it ends a stopped
/// Run, waits for a Connection while none serves, and otherwise asks the
/// work items whether to admit more, wait, or drain the Run.
async fn idled(
    (run, resolved): (&Run, &Resolved),
    (admission, state): (&mut Admission, &mut RunState),
    (servers, arrivals, in_flight): (&mut Vec<Serving>, &mut Opened, &SharedInFlight),
    (stop, starved): (Option<StopReason>, bool),
) -> Idled {
    if let Some(reason) = stop {
        end(run, admission, Ending::Stopped(reason)).await;
        return Idled::Ended;
    }
    let again = |waited| match waited {
        Waited::Ended => Idled::Ended,
        Waited::Stop(reason) => Idled::Again(Some(reason)),
        Waited::Again => Idled::Again(None),
    };
    if !servers.iter_mut().any(Serving::serves) {
        let state = (admission, state);
        return again(waited(run, state, servers, arrivals, in_flight, false).await);
    }
    match when_idle(run, admission, state).await {
        Ok(Idle::Admit) if !starved => Idled::Again(None),
        Ok(Idle::Admit | Idle::Wait) => Idled::Waits,
        Ok(Idle::Done) => {
            let state = (admission, state);
            again(drained(run, resolved, state, servers, arrivals, in_flight).await)
        }
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
            end(run, admission, failed(run, &error)).await;
            Idled::Ended
        }
    }
}

/// The delivery's loop, which returns once the Run ended or stopped, each
/// attempt it starts handing over `handing` with its own part. It owns the
/// crawl's Run state, which starts from the Session pool the run file
/// holds and from `choices`, and answers what every other task asks of it
/// (ADR 0142).
async fn deliver(
    run: &Run,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    (settings, choices): (&RunSettings, TierChoices),
    (fetching, handing): (&Fetching<'_>, &std::sync::Arc<RunHandOver>),
) {
    let Some((mut admission, mut state)) = opened(run, settings, choices, arrivals).await else {
        return;
    };
    let (in_flight, mut stop) = (&fetching.in_flight, None::<StopReason>);
    show_counted(run, &state, in_flight, []).await;
    let run_leases = leases_of(run, arrivals, &admission);
    let (mut wakes, mut queued) = (admission.wakes(), run.metrics().level(&WORK_ITEM_QUEUED));
    let lessons = Lessons::of(run, state.stored().frontier());
    let mut pages = Pages::new(run, in_flight, lessons);
    let taught = state.stored().taught();
    let mut settling = Settlements::new();
    // Resolved once, as the Run opened: no turn reads the settings again
    // (ADR 0138).
    let (resolved, read) = (settings.resolved(), Read::of(settings.resolved()));
    loop {
        if stop.is_none()
            && let Err(error) = lease_to_servers(
                run,
                &run_leases,
                read.policy,
                Pooled::of(&mut state, read.pool, (fetching, handing)),
                &mut admission,
                servers,
                &mut pages,
            )
            .await
        {
            tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be leased");
            end(run, &mut admission, failed(run, &error)).await;
            return;
        }
        let starved = run.governor().waits(WorkKind::Http);
        pages.dismiss();
        if pages.idle() && settling.is_empty() {
            let state = (&mut admission, &mut state);
            let serving = (&mut *servers, &mut *arrivals, in_flight);
            match idled((run, resolved), state, serving, (stop.take(), starved)).await {
                Idled::Ended => return,
                Idled::Again(reason) => {
                    stop = reason;
                    continue;
                }
                Idled::Waits => {}
            }
        }
        let free = stop.is_none()
            && servers.iter_mut().any(|server| {
                server.serves() && server.in_flight < read.policy.handler_concurrency
            });
        let wake = (free && !starved)
            .then(|| admission.ready_at(run.clock().now()))
            .flatten();
        let room = [stop.is_some(), free, starved];
        let shown = (resolved, read.policy);
        show(run, (&mut admission, &mut queued), shown, servers, room);
        let settles = !settling.is_empty();
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => {
                pages.shutdown().await;
                abandoned(&mut settling).await;
                admission.close().await;
                release(run, state.stored()).await;
                return;
            }
            reason = run.stopping(), if stop.is_none() => {
                stop = Some(reason);
            }
            Some(server) = arrivals.joined.recv() => {
                servers.push(Serving::joined(server));
            }
            Some(ask) = arrivals.asked.next() => {
                answered(run, (&mut admission, &mut state), in_flight, ask).await;
            }
            Some(written) = settling.next(), if settles => {
                let on = (&mut admission, servers.as_mut_slice());
                completed(run, on, (&mut state, fetching), written).await;
            }
            moved = taught.taken() => {
                merged(run, (&mut admission, &mut state), in_flight, moved).await;
            }
            ended = pages.ended() => {
                let state = (&mut state, fetching);
                let settlement = settled(run, read, state, servers, ended).await;
                settling.push(settlement);
            }
            () = sleep_until(run.clock(), wake), if wake.is_some() => {}
            () = run.governor().turn(WorkKind::Http), if free && starved => {}
            key = next_wake(wakes.as_mut()) => admission.wake(key.as_deref()),
        }
    }
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

/// Shows the crawl's requests by state as its Run state `state` counts
/// them now and, in the same change of the Run's progress, that the
/// attempts of the work items `ended` left the requests `in_flight`, their
/// outcomes written (ADR 0102; #2167).
async fn show_counted<'a>(
    run: &Run,
    state: &RunState,
    in_flight: &SharedInFlight,
    ended: impl IntoIterator<Item = &'a str>,
) {
    in_flight.lock().await.settled(run, ended, state.tally());
}

/// Answers `ask`, what another task asks of the crawl's Run state. When
/// the loop waited for its write itself, a seed, a batch of seeds or a
/// re-admission, it offers their domains' queues the work items the write
/// added, shows what it changed of the crawl's requests by state, with
/// the requests `in_flight`, and answers that the crawl may have new work.
async fn answered(
    run: &Run,
    (admission, state): (&mut Admission, &mut RunState),
    in_flight: &SharedInFlight,
    ask: Box<Question>,
) -> bool {
    let Some(added) = state.answer(ask).await else {
        return false;
    };
    for waiting in added {
        admission.enqueue(waiting);
    }
    show_counted(run, state, in_flight, []).await;
    true
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
            // The crawl's Run state opens after this job, and counts its
            // requests on from what the run file then holds.
            let recovered = &mut Changed::default();
            work_items::recover_leases(connection, &lost, retry_max, &resumed_at, recovered)?;
            let fragments = labels::Fragments::read(connection)?;
            let invariants = Invariants::read(connection, fragments.clone())?;
            Ok((
                work_items::left(connection)?,
                labels::read(connection)?,
                fragments,
                tier_choice::read(connection)?,
                (
                    code_free::read(connection)?,
                    Datasets::read(connection)?,
                    invariants,
                ),
            ))
        })
        .await;
    let left = left.map(|(left, declared, fragments, choices, declarations)| {
        let labels = labels::Declared::of(&declared, fragments, settings.resolved()).unwrap_or_else(|error| {
            tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's Labels no longer read; its requests take the Run's tier and pool");
            labels::Declared::none(settings.resolved())
        });
        (left, (labels, choices, declarations))
    });
    let (labels, choices, (extraction, datasets, invariants)) = match left {
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
    let opened = crawls.open(&run, egress, invariants, datasets).await;
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
    /// The Run's own `fetch.http_from`.
    http_from: HttpFrom,
    /// The Run's own `fetch.encoding`, a WHATWG Encoding label.
    run_encoding: Option<String>,
    /// The Run's own `crawl.content_types`.
    run_content_types: ContentTypes,
    /// Whether the Run races its requests' tiers (`fetch.tier_race`).
    races: bool,
    /// Whether the Run allows the `light_browser` tier, which its samples
    /// then also run on (`fetch.light_browser`, ADR 0020).
    light: bool,
    /// What a code-free crawl handles its requests with; none when its
    /// program does.
    code_free: Option<std::sync::Arc<CodeFree>>,
    /// The requests in flight, as the Run's progress shows them.
    in_flight: SharedInFlight,
    /// The readers of the untouched page of each Label, built once.
    untouched: Untouched,
    /// What an attempt that escalates from a Session's own transport asks
    /// the crawl's Run state through, for the Session it runs again in
    /// (#2416).
    asking: Asking,
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
            // A code-free crawl's formats are read ahead in each document's
            // hand-off to the CPU pool (issue 2189).
            formats: code_free.map(CodeFree::formats).unwrap_or_default(),
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
    /// What the attempt of `leased`, holding `lease` and watched by
    /// `watch`, hands over with its Run's pages, built once as it is
    /// leased: `of_run`, what every attempt of the Run hands over alike,
    /// with its own room beside the Run's requests, `second_copies`, its
    /// HTTP admission, what its request's Label declares, and the tier of
    /// the Session that reads its site files, `site_files`.
    fn hand_over(
        &self,
        of_run: &std::sync::Arc<RunHandOver>,
        (leased, lease, watch): (&Leased, &WorkItemLease, &Watch),
        (second_copies, fetch_admission): (SecondCopies, FetchAdmission),
        site_files: Option<&HttpTier>,
    ) -> std::sync::Arc<HandOver> {
        let label = leased.crawl_request.label.as_str();
        std::sync::Arc::new(HandOver {
            of_run: std::sync::Arc::clone(of_run),
            domain_key: leased.waiting.key.clone(),
            second_copies,
            lease: handler_lease(of_run, (leased, lease, watch), &self.in_flight),
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
            site_files: site_files.map(HttpTier::robots_txt_fetch),
        })
    }

    /// How `leased`'s attempt runs in the pool Session `session`, sending
    /// from `sends_from`, as `chosen` (its Label and the Run's `fetch.tier`
    /// chose it as its Session was drawn): a race is timed by the
    /// latencies of its Label on its domain's first requests; and what it
    /// fetches its document with, a sample's `http` run or a race's first
    /// request on `transport`'s Session when one was drawn for it (#2421);
    /// and the tier its site files are read on, from the own transport of
    /// the first of its Sessions that sends from one, the attempt's,
    /// `transport` or `site_files`, else of the attempt's Session, keeping
    /// that Session's cookies (#2422).
    fn served(
        &self,
        state: &RunState,
        leased: &Leased,
        ((session, local, sends_from), chosen): ((&SessionId, Local, SendsFrom), Chosen),
        (transport, site_files): (Option<DrawnSession>, Option<DrawnSession>),
        (second_copies, settings): (&SecondCopies, PoolSettings),
    ) -> (Tiering, Option<HttpTier>) {
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
                delay: state.race_delay(&request.label, &leased.waiting.key),
                agrees: !matches!(
                    learned,
                    AttemptPlan::On(TierChoice {
                        tier: FetchTier::Browser,
                        ..
                    })
                ),
            }),
        };
        let http = self.session_tier(session, local);
        let transport = transport.map(|drawn| TransportSession {
            http: self.session_tier(&drawn.session.id, drawn.local),
            session: drawn.session,
            in_browser: drawn.in_browser,
        });
        let site_files = site_files.map(|drawn| {
            let http = self.session_tier(&drawn.session.id, drawn.local);
            (drawn.session, http)
        });
        let reader = match (sends_from, &transport, &site_files) {
            (SendsFrom::Browser, Some(sample), _) => {
                sample.http.as_ref().map(|http| (&sample.session.id, http))
            }
            (SendsFrom::Browser, None, Some((drawn, http))) => {
                http.as_ref().map(|http| (&drawn.id, http))
            }
            (SendsFrom::Transport, _, _) | (SendsFrom::Browser, None, None) => {
                http.as_ref().map(|http| (session, http))
            }
        }
        .map(|(session, http)| http.keeping_cookies_of(session));
        let tiering = Tiering::of(
            self.tiers,
            http.as_ref(),
            (plan, self.light),
            self.labels.settings.page_check(&request.label).cloned(),
            second_copies,
        )
        .sending(sends_from, transport)
        .reading_site_files(site_files.map(|(drawn, _)| drawn))
        .escalating((sends_from == SendsFrom::Transport).then(|| {
            let pool = self.labels.pools.pool(&request.label);
            Escalating {
                asking: self.asking.clone(),
                labels: std::sync::Arc::clone(&self.labels),
                pool,
                settings,
            }
        }));
        (tiering, reader)
    }

    /// The Run's HTTP tier as the pool Session `session`, holding `local`,
    /// sends on it (ADR 0044), through the Session's own proxy when the Run
    /// sets one (#1640): `None`, which fails the attempt's fetch, when the
    /// Session's addresses are no longer held or its transport cannot be
    /// built, rather than sending from an address the OS chooses.
    fn session_tier(&self, session: &SessionId, local: Local) -> Option<HttpTier> {
        let http = self.http?;
        let source = match local {
            None => None,
            Some(Ok(addresses)) => Some(LocalSource::new(addresses, self.interface.clone())),
            Some(Err(unavailable)) => {
                tracing::warn!(entry = %unavailable.entry, "a Session's local addresses are no longer held; its HTTP fetches fail");
                return None;
            }
        };
        http.of_session(session, source.as_ref())
            .inspect_err(|error| tracing::warn!(%error, "a Session's HTTP transport could not be built; its HTTP fetches fail"))
            .ok()
    }

    /// Closes the browser contexts of the retired Sessions of `pool` no
    /// attempt holds any more, for the call `context`.
    async fn retire(&self, run: &Run, pool: &mut SessionPool, context: &CallContext) {
        for session in pool.closable() {
            if let Some(http) = self.http {
                http.forget_session(&session).await;
            }
            if let Err(error) = self.tiers.retire_browser(run.id(), &session, context).await {
                tracing::error!(%error, run_id = %run.id(), session_id = %session.0, "a retired Session's browser context did not close");
            }
        }
    }
}

/// What each attempt is given: a Session drawn from the crawl's Session
/// pool under the Run's settings now, the tier that fetches its request's
/// document, and what every attempt of the Run hands over alike.
struct Pooled<'a> {
    state: &'a mut RunState,
    settings: PoolSettings,
    fetching: &'a Fetching<'a>,
    handing: &'a std::sync::Arc<RunHandOver>,
}

impl<'a> Pooled<'a> {
    /// Draws from `state`'s pool under `settings`, fetching on `fetching`,
    /// each attempt handing over `handing` with its own part.
    fn of(
        state: &'a mut RunState,
        settings: PoolSettings,
        (fetching, handing): (&'a Fetching<'a>, &'a std::sync::Arc<RunHandOver>),
    ) -> Self {
        Self {
            state,
            settings,
            fetching,
            handing,
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
    pages: &mut Pages,
) -> Result<(), StateError> {
    for (place, server) in servers.iter_mut().enumerate() {
        if !server.serves() || server.in_flight >= policy.handler_concurrency {
            continue;
        }
        // A Run with no work item ready asks the governor for nothing, and
        // passes any turn it waits for to the Runs that wait.
        let now = run.clock().now();
        if admission.ready_at(now).is_none_or(|ready| ready > now) {
            run.governor().pass(WorkKind::Http);
            return Ok(());
        }
        let mut permits: Vec<Permit> =
            std::iter::from_fn(|| run.governor().try_admit(WorkKind::Http))
                .take(policy.handler_concurrency - server.in_flight)
                .collect();
        if permits.is_empty() {
            return Ok(());
        }
        let served = admit(run, admission, permits.len(), &mut pooled).await?;
        let pool = pooled.state.pool();
        for rotated in pool.rotated_within_block() {
            warn_rotated(run, rotated).await;
        }
        pooled.fetching.retire(run, pool, &server.context).await;
        for drawn in served {
            let Drawn {
                leased,
                session:
                    DrawnSession {
                        session,
                        in_browser,
                        local,
                        sends_from,
                    },
                chosen,
                transport,
                site_files,
            } = drawn;
            let lease = run_leases.grant(
                &leased.crawl_request.id,
                leased.lease_epoch,
                policy.lease_length(),
            );
            server.in_flight += 1;
            let copies = admission.second_copies();
            let fetching = &pooled.fetching;
            let (tiering, reader) = fetching.served(
                pooled.state,
                &leased,
                ((&session.id, local, sends_from), chosen),
                (transport, site_files),
                (&copies, pooled.settings),
            );
            let admission = FetchAdmission::holding(
                permits
                    .pop()
                    .expect("invariant: one admission per work item admitted"),
            );
            let watch = Watch::new(run.metrics());
            let hand_over = fetching.hand_over(
                pooled.handing,
                (&leased, &lease, &watch),
                (copies, admission.clone()),
                reader.as_ref(),
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
            pages.start(Box::new(Start {
                server: (place, server.context.clone()),
                leased: (leased, (session, in_browser), leased_count(run)),
                lease,
                watch,
                fetching: (tiering, (hand_over, fetching.code_free.clone())),
                admission,
            }));
        }
        // Fewer were ready than the governor admitted: each admission left
        // goes back, and the Run's turn to the Runs that wait.
        if !permits.is_empty() {
            drop(permits);
            run.governor().pass(WorkKind::Http);
            return Ok(());
        }
    }
    Ok(())
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
/// That job first gives back each work item still leased, whose attempt
/// ended with its outcome unwritten: retried, it is delivered, and the
/// crawl does not finish over it. That job, or a batch of its own while
/// the crawl is kept alive, also
/// writes the tier choices' counts `state` holds that the run file has not
/// heard of: no lease follows the last draws to carry them (ADR 0142).
/// A crawl kept alive seals the open batch before it waits, so what it
/// did is in the file while it waits (ADR 0142).
/// What that job changed of the crawl's requests by state is counted and
/// shown with the requests `in_flight`.
async fn drained(
    run: &Run,
    resolved: &Resolved,
    (admission, state): (&mut Admission, &mut RunState),
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    in_flight: &SharedInFlight,
) -> Waited {
    let keeps_alive: bool = resolved.get(&CRAWL_KEEP_ALIVE);
    let counted = state.counted();
    if keeps_alive && !run.is_let_finish() {
        if !counted.is_empty()
            && let Err(error) = state.stored().count(counted).await
        {
            tracing::warn!(%error, run_id = %run.id(), "the tier choices' counts were not written");
        }
        // It waits for new work, for as long as none comes: what it did is
        // sealed into the file first, so a reader sees it now rather than
        // once the open batch's interval passed (ADR 0142).
        if let Err(error) = state.seal().await {
            tracing::warn!(%error, run_id = %run.id(), "the open batch was not sealed");
        }
    } else {
        let lost = AttemptFailure::outcome_not_written("its attempt ended and left it leased");
        let unwritten = Failure {
            code: lost.code,
            message: lost.message,
            details: lost.details,
        };
        let retry_max = CrawlPolicy::of(resolved).retry_max;
        let ended_at = run.point_in_time();
        // Its answer promises nothing outside (#2117): the Run's end, which
        // follows it, syncs, and so does a later call's refusal, as its own
        // write commits after the close.
        let closed = state
            .stored()
            .close_if_drained(counted, (unwritten, retry_max), ended_at)
            .await;
        let ending = match closed {
            Ok((short, changed)) => {
                state.changed(changed);
                show_counted(run, state, in_flight, []).await;
                match short {
                    Some(reasons) => Ending::Finished(reasons),
                    // A work item it gave back is pending in a batch the
                    // next reads must see (ADR 0142).
                    None => match state.seal().await {
                        Ok(()) => return Waited::Again,
                        Err(error) => failed(run, &error),
                    },
                }
            }
            Err(error) => {
                tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                failed(run, &error)
            }
        };
        end(run, admission, ending).await;
        return Waited::Ended;
    }
    progress::show_waits(run, &[], resolved, run.clock().now(), Room::None);
    progress::show_idle(run, true);
    waited(run, (admission, state), servers, arrivals, in_flight, true).await
}

/// Waits, with nothing to deliver now, until the Run stops at once, is
/// asked to stop at its next safe point, a Connection joins to serve it, a
/// seed is added or failures are re-admitted or, `until_finish`,
/// `run.finish` lets it finish; meanwhile it answers what other tasks ask
/// of the crawl's Run state, `state`, and shows what a write it waited
/// for changed of the crawl's requests by state, with the requests
/// `in_flight`.
async fn waited(
    run: &Run,
    (admission, state): (&mut Admission, &mut RunState),
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    in_flight: &SharedInFlight,
    until_finish: bool,
) -> Waited {
    loop {
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => {
                admission.close().await;
                release(run, state.stored()).await;
                return Waited::Ended;
            }
            reason = run.stopping() => return Waited::Stop(reason),
            () = run.let_finish(), if until_finish => return Waited::Again,
            Some(server) = arrivals.joined.recv() => {
                servers.push(Serving::joined(server));
                return Waited::Again;
            }
            Some(ask) = arrivals.asked.next() => {
                if answered(run, (&mut *admission, &mut *state), in_flight, ask).await {
                    return Waited::Again;
                }
            }
        }
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
/// again from the pending work items when they hold none ready, once any
/// work item an admission left with no domain key is keyed. Its reads
/// decide with no seal: each work item an answered write changed in the
/// open batch is known in memory beside them (ADR 0142).
async fn when_idle(
    run: &Run,
    admission: &mut Admission,
    state: &mut RunState,
) -> Result<Idle, StateError> {
    if admission.ready_at(run.clock().now()).is_some() {
        return Ok(Idle::Wait);
    }
    key_new(state.stored(), admission, Unkeyed::Running).await?;
    fill(state, admission, None).await?;
    if admission.ready_at(run.clock().now()).is_some() {
        return Ok(Idle::Admit);
    }
    Ok(if state.any_pending().await? {
        Idle::Wait
    } else {
        Idle::Done
    })
}

/// What settling one ended attempt keeps of it once its Session is scored,
/// for its Connection's place, its domain's turn and queue after its
/// outcome is written.
struct Settling {
    server: usize,
    waiting: Waiting,
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

/// The settlements whose outcomes the run state's writer has yet to
/// answer, which the delivery loop polls beside its other work and never
/// waits for alone (#2168).
type Settlements = FuturesUnordered<Settlement>;

/// A group of ended attempts whose outcomes are queued with the run
/// state's writer: once each is answered, what each one's settling keeps
/// and what its outcome left the delivery to do, in their order.
type Settlement = Pin<Box<dyn Future<Output = (Vec<Settling>, Vec<Finished>)> + Send>>;

/// Settles the attempts that ended, `ended`, in the order they ended,
/// under the Run's policy and pool settings, resolved as it opened (ADR
/// 0138): scores each one's Session; queues each one's outcome as one job of the run state's
/// writer, in their order, so the writer takes them in one group rather
/// than the loop waiting for each in turn (#1765), and each one's failure
/// rolls back its own alone (ADR 0138). It answers the group's
/// settlement, which the loop polls beside its other work rather than
/// waiting for it (#2168): at `batch` an outcome rides the writer's open
/// batch, and no page waits for the writer. [`completed`] finishes it.
async fn settled(
    run: &Run,
    read: Read,
    (state, fetching): (&mut RunState, &Fetching<'_>),
    servers: &mut [Serving],
    ended: Vec<Attempt>,
) -> Settlement {
    let (attempts, settlings, scores) = scored(run, read, &mut *state, servers, ended);
    for attempt in &attempts {
        fetching
            .retire(run, state.pool(), &servers[attempt.server].context)
            .await;
    }
    let mut queued = Vec::with_capacity(attempts.len());
    for ((attempt, settling), scored) in attempts.into_iter().zip(&settlings).zip(scores) {
        let held = settling.held.is_some();
        let on = (run, state.stored());
        queued.push(queue_outcome(on, read.policy, held, attempt, scored).await);
    }
    let stored = state.stored().clone();
    Box::pin(async move {
        let finished = join_all(queued.into_iter().map(|queued| queued.finished(&stored))).await;
        (settlings, finished)
    })
}

/// Finishes a settlement once its outcomes were written: frees each
/// attempt's place on its Connection, which its page holds until then so
/// that a Connection's handler concurrency counts the pages it has not
/// settled and a retried work item goes back before its place is leased
/// again; counts what they
/// changed of the crawl's requests by state and shows it with the
/// attempts out of the requests in flight, as one change of the Run's
/// progress (#2167); then returns each one's domain turn and puts each
/// retried work item back in its queue, in their order, offers its
/// domain's queue each work item an outcome added, keyed as it was added,
/// and journals what the queues changed.
async fn completed(
    run: &Run,
    (admission, servers): (&mut Admission, &mut [Serving]),
    (state, fetching): (&mut RunState, &Fetching<'_>),
    (settlings, finished): (Vec<Settling>, Vec<Finished>),
) {
    let mut retried = Vec::with_capacity(finished.len());
    let mut keyed = Vec::new();
    for (outcome, settling) in finished.into_iter().zip(&settlings) {
        servers[settling.server].in_flight -= 1;
        state.settled(outcome.leased, outcome.retried);
        keyed.extend(state.added(outcome.keyed));
        retried.push(outcome.retried);
        state.changed(outcome.changed);
    }
    let ended = settlings
        .iter()
        .map(|settling| settling.waiting.id.as_str());
    show_counted(run, state, &fetching.in_flight, ended).await;
    returned(run, admission, settlings.into_iter().zip(retried)).await;
    for waiting in keyed {
        admission.enqueue(waiting);
    }
    journal(run, state.stored(), admission).await;
}

/// Waits for every settlement in `settling`, finishing none: the crawl was
/// cancelled, and its leases are released next.
async fn abandoned(settling: &mut Settlements) {
    while settling.next().await.is_some() {}
}

/// Marks the Connection of each attempt of `ended` gone when its client
/// went, and scores its Session in
/// `state`'s pool under what the loop `read`, in their order, answering
/// them with what each one's settling keeps and what its outcome writes of
/// its score and tier choice. Each stays among the requests in flight its
/// Run's progress shows until its outcome is written.
fn scored(
    run: &Run,
    read: Read,
    state: &mut RunState,
    servers: &mut [Serving],
    ended: Vec<Attempt>,
) -> (Vec<Attempt>, Vec<Settling>, Vec<Scored>) {
    let mut attempts = Vec::with_capacity(ended.len());
    let mut settlings = Vec::with_capacity(ended.len());
    let mut scores = Vec::with_capacity(ended.len());
    for mut attempt in ended {
        let mut chosen = attempt.sampled.take().map(|(sampled, weighed)| {
            let unit = state.classify(&sampled, weighed, wall_ms(run));
            (sampled.label, sampled.domain_key, unit)
        });
        // A refusal of the transport drops its Label's choice on the
        // domain, which its outcome writes (#2419): never a sample's.
        if attempt.refused {
            let (label, domain_key) = (
                &attempt.leased.crawl_request.label,
                &attempt.leased.waiting.key,
            );
            chosen = state
                .invalidate(label, domain_key)
                .map(|unit| (label.clone(), domain_key.clone(), unit));
        }
        if let Some((raced, latency)) = attempt.raced.take() {
            state.time(&raced.label, &raced.domain_key, latency);
        }
        let pool = state.pool();
        if let Answer::ClientGone = attempt.answer {
            servers[attempt.server].gone = true;
        }
        let waiting = attempt.leased.waiting.clone();
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
        // An attempt that escalated from its Session's own transport ran
        // again in a Session of the browser, whose answer is the attempt's:
        // the first met what only a browser serves, which says nothing of
        // how its client fares. One its site refused on the transport
        // retires its Session and moves no proxy tier (#2419).
        let effect = if attempt.refused {
            Effect::TransportRefused
        } else {
            effect(&attempt.answer)
        };
        // The Session whose run answered takes the score, and every other
        // one the attempt drew none: a sample's `http` run sent from a
        // Session of its own, whose attempt's answer is its browser run's,
        // or a race's first request sent from one, unless its answer is
        // the attempt's (#2421).
        let (on_session, on_transport) = match &attempt.escalated {
            Some(escalated) => {
                pool.settle(escalated, &waiting.key, effect, &read.pool);
                (Effect::Neutral, Effect::Neutral)
            }
            None if attempt.transport_answered => (Effect::Neutral, effect),
            None => (effect, Effect::Neutral),
        };
        pool.settle(&attempt.session, &waiting.key, on_session, &read.pool);
        if let Some(transport) = &attempt.transport {
            pool.settle(transport, &waiting.key, on_transport, &read.pool);
        }
        // The Session drawn only to read the attempt's site files: what
        // they answer says nothing of how its client fares (#2422).
        if let Some(reader) = &attempt.site_files {
            pool.settle(reader, &waiting.key, Effect::Neutral, &read.pool);
        }
        let held = held_past_bound(
            &attempt.answer,
            attempt.handed_at.unwrap_or_else(|| run.clock().wall()),
            read.longest_wait,
        );
        settlings.push(Settling {
            server: attempt.server,
            waiting,
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
/// past the Run's longest wait, and puts its work item back in its queue
/// when it was `retried`.
async fn returned(
    run: &Run,
    admission: &mut Admission,
    settled: impl Iterator<Item = (Settling, bool)>,
) {
    for (settling, retried) in settled {
        let Settling {
            waiting,
            domain,
            held,
            ..
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
    }
}

/// Opens the Run's admission and its Run state, which starts from the
/// Session pool the run file holds and from `choices`, and admits within
/// what `arrivals` opened the crawl with, or ends the Run as failed when
/// they could not open.
async fn opened(
    run: &Run,
    settings: &RunSettings,
    choices: TierChoices,
    arrivals: &Opened,
) -> Option<(Admission, RunState)> {
    match open(run, settings, arrivals).await {
        Ok((admission, (pool, tally), stored)) => {
            Some((admission, RunState::new(pool, choices, tally, stored)))
        }
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's admission could not open");
            run.end(failed(run, &error)).await;
            None
        }
    }
}

/// Opens the Run's admission, the gate Foxhound's composition passes in,
/// and what its Run state reads and writes of the run file, each work
/// item it adds with no attempt keyed as that gate keys it; keys the work
/// items not keyed yet and fills each domain's queue; and opens its
/// Session pool from the run file, on an `Entropy` of its own, with the
/// requests by state its Run state counts on from.
async fn open(
    run: &Run,
    settings: &RunSettings,
    arrivals: &Opened,
) -> Result<(Admission, (SessionPool, Tally), Stored), String> {
    let resolved = settings.resolved();
    let mut admission = run
        .admission(resolved)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("the composition passes in no gate in admission")?;
    let within = Within {
        egress: std::sync::Arc::clone(&arrivals.egress),
        invariants: arrivals.invariants.clone(),
        staging: arrivals.staging.clone(),
        intake: arrivals.intake.clone(),
    };
    let stored = Stored::new(
        run.state().clone(),
        within,
        (admission.keys(), run.clock()),
        run.metrics(),
    );
    stored
        .fill_memory()
        .await
        .map_err(|error| error.to_string())?;
    key_new(&stored, &mut admission, Unkeyed::Opening)
        .await
        .map_err(|error| error.to_string())?;
    // As the crawl opens it has claimed nothing: the run file holds every
    // pending work item.
    for waiting in stored
        .waiting(None, admission.bound())
        .await
        .map_err(|error| error.to_string())?
    {
        admission.enqueue(waiting);
    }
    let (pool, tally) = stored
        .open(run.fork_entropy().await)
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
    Ok((admission, (pool, tally), stored))
}

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

/// How many work items not keyed yet [`key_new`] reads at a time, so the
/// seeds of a start never sit in memory whole.
const KEYED_AT_ONCE: usize = 1024;

/// Keys each work item not keyed yet, `found` as the crawl opens or once
/// it is idle. A start keys its seeds in the write that adds them, as its
/// gate keys them (#2168), and every work item added once the crawl runs
/// is keyed in the write that adds it, a seed, a re-admission, an
/// attempt's links and a sitemap's entries (#2167), so a crawl finds none
/// unless a start ran with no gate; one an admission left so would wait
/// outside every domain's queue for ever, and is keyed here, in a write
/// the Run state counts. It offers each to its domain's queue, which
/// takes it unless full; one it refuses waits in run state until its queue
/// fills again. At a level that batches attempts a round's keys ride the
/// open batch, which the next read would not see, so each round that keyed
/// any seals it (ADR 0142): the next round and the reads after it see them.
async fn key_new(
    stored: &Stored,
    admission: &mut Admission,
    found: Unkeyed,
) -> Result<(), StateError> {
    loop {
        let unkeyed = stored.unkeyed(KEYED_AT_ONCE).await?;
        let read = unkeyed.len();
        if read == 0 {
            return Ok(());
        }
        key(stored, admission, unkeyed, found).await?;
        stored.seal().await?;
        if read < KEYED_AT_ONCE {
            return Ok(());
        }
    }
}

/// Keys each of `unkeyed`, as [`key_new`] does.
async fn key(
    stored: &Stored,
    admission: &mut Admission,
    unkeyed: Vec<(String, Waiting)>,
    found: Unkeyed,
) -> Result<(), StateError> {
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
    stored.key(keys, found).await?;
    for waiting in keyed {
        admission.enqueue(waiting);
    }
    Ok(())
}

/// Fills the queue of `key`, or of every key when `None`, from the pending
/// work items `state` reads, at most its bound.
async fn fill(
    state: &mut RunState,
    admission: &mut Admission,
    key: Option<String>,
) -> Result<(), StateError> {
    for waiting in state.waiting(key, admission.bound()).await? {
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
) -> Result<Vec<Drawn>, StateError> {
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
            Err(error) => return Err(error.into()),
        }
        if admitted.drained {
            drained.push(admitted.key.clone());
        }
        ids.push(admitted.id);
        keys.push(admitted.key);
    }
    if !refused.is_empty() {
        // Each ended in a batch a read may not see yet.
        let unsent: Vec<String> = refused
            .iter()
            .map(|(admitted, _)| admitted.id.clone())
            .collect();
        let changed = end_unsent(run, pooled.state.stored(), admission, refused).await?;
        pooled.state.changed(changed);
        pooled.state.gone(unsent);
        show_counted(run, pooled.state, &pooled.fetching.in_flight, []).await;
    }
    let asked: Vec<(String, String)> = ids.into_iter().zip(keys).collect();
    let drawn = if asked.is_empty() {
        Vec::new()
    } else {
        leased_and_drawn(run, charges, asked.clone(), pooled).await?
    };
    // A work item a rule learned later merged after the gate took it is
    // no longer pending: it gives its domain's turn back unsent.
    for (id, key) in &asked {
        if !drawn
            .iter()
            .any(|drawn| drawn.leased.crawl_request.id == *id)
        {
            admission.ended(id, key, now, Ended::Done).await;
        }
    }
    for key in drained {
        fill(pooled.state, admission, Some(key)).await?;
    }
    Ok(drawn)
}

/// Leases each pending work item of `asked`, by ID with its domain key, in
/// `pooled`'s Run state, whose claim journals `charges`, and draws the
/// Session that serves each and how its tier is chosen (ADR 0142).
async fn leased_and_drawn(
    run: &Run,
    charges: UnjournaledCharges,
    asked: Vec<(String, String)>,
    pooled: &mut Pooled<'_>,
) -> Result<Vec<Drawn>, StateError> {
    let fetching = pooled.fetching;
    let drawing = Drawing {
        labels: &fetching.labels,
        settings: pooled.settings,
        run_tier: fetching.run_tier,
        http_from: fetching.http_from,
        races: fetching.races,
        reads_site_files: fetching.http.is_some()
            && pooled
                .handing
                .site_preferences
                .reads_robots_txt(goodfetch_fetch_tier::Request::CrawlRequest),
        wall: (run.clock().wall(), wall_ms(run)),
    };
    pooled.state.lease(run, charges, asked, &drawing).await
}

/// Writes what the gate changed into the run file.
async fn journal(run: &Run, stored: &Stored, admission: &mut Admission) {
    let clock = run.clock();
    if let Some(journal) = admission.journal(clock.wall(), clock.now())
        && let Err(error) = stored.journal(journal).await
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
    /// The retry rule its attempt ends under when its job fails.
    policy: CrawlPolicy,
    /// Whether it commits its answer.
    commits: bool,
    written: QueuedEnd,
}

/// What an attempt's outcome left the delivery to do once it was written:
/// its work item as its lease took it, each work item it added keyed, in
/// order, as its first lease takes it, whether its work item waits again
/// for a retry, and what its job changed of the crawl's requests by state.
struct Finished {
    leased: Leased,
    keyed: Vec<Leased>,
    retried: bool,
    changed: Changed,
}

/// Queues `attempt`'s outcome as one job of the run state's writer (ADR
/// 0138), with what its Session's score changed and its tier choice,
/// `scored`: its Records committed once while its lease holds, its
/// links admitted and keyed, or its handler's skip, its block, or its work
/// item retried or failed, under the Run's `policy`, with no retry when its
/// failure's site asked for a time further away than the Run waits
/// (`held`), or its sitemap's entries admitted and keyed, the fetch ended
/// with no outcome (ADR 0093).
async fn queue_outcome(
    (run, file): (&Run, &Stored),
    policy: CrawlPolicy,
    held: bool,
    attempt: Attempt,
    scored: Scored,
) -> QueuedOutcome {
    let Attempt {
        leased,
        lease,
        answer,
        refused: transport_refused,
        ..
    } = attempt;
    let (work_item, lease_epoch) = (leased.crawl_request.id.clone(), leased.lease_epoch);
    let masks = run.secrets().mask().masks().clone();
    let tries = leased.tries;
    let on = (run, file, policy);
    let failure = move |failure, why: &Why, retryable| {
        retried_or_failed(policy, tries, failure, why, retryable)
    };
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
        Answer::RobotsTxtUnreachable(message) | Answer::HopsRobotsTxtUnreachable(message)
            if policy.may_retry(leased.tries) =>
        {
            Unsuccessful::Retried(Failure::new(SITE_PREFERENCES_REFUSED.name, &message))
        }
        Answer::RobotsTxtUnreachable(_) | Answer::HopsRobotsTxtUnreachable(_) => {
            refused(SkipReason::RobotsTxt)
        }
        Answer::SitemapRead(document) => {
            let ending = file.sitemap_read((work_item, lease_epoch), document);
            return queued(on, (leased, lease), (scored, ending), false).await;
        }
        Answer::Handled(handled) => {
            match lease.committing(run.state(), &masks, handled.records, run.point_in_time()) {
                Ok(commit) => {
                    commits = true;
                    let refused: Refusing =
                        Box::new(move |refusal: goodfetch_engine::CallError| {
                            let (kept, why, retryable) = refused_commit(&masks, &refusal);
                            failure(kept, &why, retryable)
                        });
                    let ending = attempt_outcome::Ending::Commit { commit, refused };
                    return queued(on, (leased, lease), (scored, ending), commits).await;
                }
                Err(_) => failure(
                    Failure::new(LEASE_LOST.name, LEASE_LOST.description),
                    &Why::Other,
                    LEASE_LOST.retryable,
                ),
            }
        }
        Answer::ClientGone => failure(
            Failure::new(CLIENT_GONE.name, CLIENT_GONE.description),
            &Why::Other,
            CLIENT_GONE.retryable,
        ),
        Answer::LeaseEnded => failure(
            Failure::new(HANDLER_LOST.name, HANDLER_LOST.description),
            &Why::Other,
            HANDLER_LOST.retryable,
        ),
        Answer::Failed(failed) => {
            let failed = *failed;
            let (why, retryable) = (failed.why().clone(), failed.retryable);
            let kept = kept_failure(&masks, failed.code, &failed.message, failed.details);
            match why {
                // A refusal of the transport runs again as a sample of its
                // Label, whose comparison may move it off the transport
                // (#2419).
                Why::Blocked(_) if transport_refused && policy.may_retry(tries) => {
                    Unsuccessful::Retried(kept)
                }
                Why::Blocked(cause) => Unsuccessful::Blocked(cause),
                _ if held => Unsuccessful::Failed(kept.with_fix(RETRY_AFTER_FIX)),
                why => failure(kept, &why, retryable),
            }
        }
    };
    let ending = attempt_outcome::Ending::End {
        work_item,
        lease_epoch,
        unsuccessful,
        ended_at: run.point_in_time(),
    };
    queued(on, (leased, lease), (scored, ending), commits).await
}

/// Queues the outcome of `leased`'s attempt under `lease`, ending as
/// `ending` says beside `scored`, in the run state's writer's queue now.
async fn queued(
    (run, file, policy): (&Run, &Stored, CrawlPolicy),
    (leased, lease): (Leased, WorkItemLease),
    (scored, ending): (Scored, attempt_outcome::Ending),
    commits: bool,
) -> QueuedOutcome {
    let outcome = Outcome {
        scored: scored.changes,
        chosen: scored.chosen,
        ending,
    };
    let written = file.end(outcome).await;
    QueuedOutcome {
        leased,
        lease,
        run: run.clone(),
        policy,
        commits,
        written,
    }
}

impl QueuedOutcome {
    /// Waits for its job's answer, then releases its lease and finishes
    /// what the job committed: a committed attempt's suspected trap links'
    /// Provenance and the files it took out, and the failure an attempt
    /// ended with counted in its Run's outcomes. A job that failed ends
    /// its attempt as one whose outcome was not written, in a job of its
    /// own ([`outcome_lost`]): its work item never stays leased.
    async fn finished(self, stored: &Stored) -> Finished {
        let Self {
            leased,
            lease,
            run,
            policy,
            commits,
            written,
        } = self;
        let mut finished = Finished {
            leased: leased.clone(),
            keyed: Vec::new(),
            retried: false,
            changed: Changed::default(),
        };
        let ended = match written.await {
            Ok(written) => {
                finished.changed = written.changed;
                written.ended
            }
            Err(error) => {
                let cause = error.to_string();
                if commits {
                    // Traced as a commit the run file refused.
                    if let Err(CommitError::RunState(error)) =
                        lease.committed(Err(error.run_file()))
                    {
                        tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                    }
                } else {
                    tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                }
                (finished.retried, finished.changed) =
                    outcome_lost((&run, stored), policy, &leased, &cause).await;
                lease.release();
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
                match read {
                    Some(keyed) => finished.keyed = keyed,
                    None => {
                        tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "a sitemap's fetch past its lease reads nothing");
                    }
                }
            }
        }
        finished
    }
}

/// Ends the attempt of `leased` whose outcome's job failed for `cause`,
/// in a job of its own: retried while `policy` leaves it retries, failed
/// with that cause otherwise, as any attempt that failed. Answers whether
/// it waits again for a retry, and what the job changed of the crawl's
/// requests by state. When this job fails too, its work item stays leased
/// until the crawl's finish, which gives it back ([`drained`]).
async fn outcome_lost(
    (run, stored): (&Run, &Stored),
    policy: CrawlPolicy,
    leased: &Leased,
    cause: &str,
) -> (bool, Changed) {
    let lost = AttemptFailure::outcome_not_written(cause);
    let (why, retryable) = (lost.why().clone(), lost.retryable);
    let masks = run.secrets().mask().masks().clone();
    let kept = kept_failure(&masks, lost.code, &lost.message, lost.details);
    let unsuccessful = retried_or_failed(policy, leased.tries, kept, &why, retryable);
    let attempt = (leased.crawl_request.id.clone(), leased.lease_epoch);
    let ended = stored
        .end_lost(attempt, unsuccessful, run.point_in_time())
        .await;
    match ended {
        Ok((
            attempt_outcome::Ended::Unsuccessful {
                ended: true,
                recorded,
                retried,
            },
            changed,
        )) => {
            if let Some(code) = recorded {
                run.record_outcome(&code);
            }
            (retried, changed)
        }
        Ok((_, changed)) => (false, changed),
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), work_item = %leased.crawl_request.id, "an attempt whose outcome was not committed was not ended: its work item stays leased until the crawl's finish");
            (false, Changed::default())
        }
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

/// The failure of a commit the run file refused with `refusal`, as the
/// Run's state keeps it, with why and whether it is retryable.
fn refused_commit(
    masks: &goodfetch_engine::mask::Masks,
    refusal: &goodfetch_engine::CallError,
) -> (Failure, Why, bool) {
    let refusal = AttemptFailure::entered(
        refusal.code().to_owned(),
        refusal.retryable(),
        refusal.message().to_owned(),
        refusal.details().cloned().unwrap_or_default(),
    );
    let (why, retryable) = (refusal.why().clone(), refusal.retryable);
    let kept = kept_failure(masks, refusal.code, &refusal.message, refusal.details);
    (kept, why, retryable)
}

/// A request the site's preferences refused before its handler ran,
/// skipped with `reason` (ADR 0086).
fn refused(reason: SkipReason) -> Unsuccessful {
    Unsuccessful::Skipped {
        reason,
        message: None,
    }
}

/// How `failure`, which failed for `why`, on a work item's `attempt`th
/// attempt since its latest admission ends it under `policy`: retried
/// while it is `retryable` and retries are left, failed otherwise; a
/// handler's own failure is retryable only as `crawl.retry.handler_failed`
/// says, and one the engine's panic ended only as
/// `crawl.retry.engine_panicked` says (ADR 0148).
fn retried_or_failed(
    policy: CrawlPolicy,
    attempt: u32,
    failure: Failure,
    why: &Why,
    retryable: bool,
) -> Unsuccessful {
    let retryable = match why {
        Why::Handler => policy.retry_handler_failed,
        Why::Panicked => policy.retry_engine_panicked,
        _ => retryable,
    };
    if retryable && policy.may_retry(attempt) {
        Unsuccessful::Retried(failure)
    } else {
        Unsuccessful::Failed(failure)
    }
}

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run, stored: &Stored) {
    if let Err(error) = stored.release().await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
