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
//! request goes for it. Once every work item
//! is done, it finishes the Run; with `crawl.keep_alive` (RT-12) it waits
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

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_engine::CallContext;
use goodfetch_engine::admission::{Admission, Ended, SecondCopies, Waiting, Wakes};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::ledger::Refusal;
use goodfetch_engine::local_addresses::{
    BlockSizes, LocalAddresses, Rotation, SessionAddresses, System, Unavailable,
};
use goodfetch_engine::mask::Scope;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::runs::{ChargeError, Ending, Run};
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_fetch_tier::{
    Choosing, ContentTypes, FetchTiers, HttpTier, LocalSource, PooledSession, RobotsTxtFetch,
    SetTier, SignalFiles, TierChoice,
};
use goodfetch_frontier::Bounds;
use goodfetch_protocol::egress::EGRESS_LOCAL_ADDRESSES;
use goodfetch_protocol::foxhound::crawl::{
    CLIENT_GONE, CRAWL_CONTENT_TYPES, CRAWL_KEEP_ALIVE, CrawlRequestHandled, HANDLER_FAILED,
    HANDLER_LOST, LEASE_LOST, WORK_ITEM_RETRIED,
};
use goodfetch_protocol::foxhound::fetch::{
    FETCH_ENCODING, FETCH_SAMPLE_COUNT, FETCH_SAMPLE_REFRESH_AFTER, FETCH_TIER_RACE, FetchTier,
    SampleRefresh,
};
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::report::LOCAL_ADDRESS_ROTATED_WITHIN_BLOCK;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::SITE_PREFERENCES_REFUSED;
use goodfetch_protocol::page::proxy::{PROXY_PROVIDER, PROXY_SERVER};
use goodfetch_protocol::run::{
    EndingError, NothingDone, RUN_STATE_UNAVAILABLE, RunDetails, RunWarning, StopReason,
};
use goodfetch_protocol::usage::REQUESTS;
use goodfetch_session_pool::{LocalAddressing, PoolSettings, Rotated, SessionPool};
use serde_json::{Map, Value};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::code_free::{self, CodeFree};
use crate::completeness;
use crate::labels;
use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::{CrawlPolicy, resumed_domain_policy};
use crate::progress::{self, InFlight, Room, SharedInFlight};
use crate::serving::{Crawls, Opened, Server};
use crate::tier_choice::{self, AttemptPlan, Sampling, TierChoices};
use crate::tier_race::{Latencies, Raced};
use crate::work_items::{self, Failure, Leased, Unsuccessful};
use goodfetch_protocol::foxhound::progress::{InFlightPhase, InFlightRequest};

mod attempt;
mod politeness;

use attempt::{Answer, Attempt, Gating, Tiering, attempt, block_cause, effect, leased_count};
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
    let races = settings.resolved().boolean(FETCH_TIER_RACE.name) == Some(true);
    // One HTTP tier for the Run, so its hops share connections (#763); the
    // navigation gate fetches robots.txt on it too, from the first
    // Session's local addresses under `egress.local_addresses` (ADR 0044).
    let interface = local_interface(settings.resolved());
    let built = match run_source(settings.resolved(), interface.as_ref()) {
        Ok(local) => FetchTiers::built(&run, settings.resolved(), local).await,
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
    let fetching = Fetching {
        tiers: &tiers,
        http: http.as_ref(),
        interface,
        robots: http.as_ref().map(HttpTier::robots_txt_fetch),
        egress: std::sync::Arc::clone(&arrivals.egress),
        labels: &labels,
        run_tier,
        run_encoding: settings
            .resolved()
            .value(FETCH_ENCODING.name)
            .as_str()
            .map(str::to_owned),
        // Its shape was checked when the crawl started.
        run_content_types: ContentTypes::of(settings.resolved().value(CRAWL_CONTENT_TYPES.name))
            .unwrap_or_default(),
        choices: tokio::sync::Mutex::new(choices.sampled_as(sampling(settings.resolved()))),
        races,
        light: FetchTiers::light_allowed(settings.resolved()),
        latencies: tokio::sync::Mutex::new(Latencies::default()),
        in_flight: std::sync::Arc::new(tokio::sync::Mutex::new(InFlight::default())),
        code_free: code_free.map(std::sync::Arc::new),
        signal_files: SignalFiles::default(),
    };
    deliver(&run, &mut servers, &mut arrivals, &settings, &fetching).await;
    crawls.close(run.id()).await;
}

/// How the Run samples its tier choices: `fetch.sample.count` and
/// `fetch.sample.refresh_after`, the built-in where a value no longer
/// reads (the settings checked both at the start).
fn sampling(resolved: &Resolved) -> Sampling {
    let built_in = Sampling::default();
    let refresh = SampleRefresh::read(resolved.value(FETCH_SAMPLE_REFRESH_AFTER.name)).ok();
    Sampling {
        count: resolved
            .value(FETCH_SAMPLE_COUNT.name)
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
/// staging.
fn leases_of(run: &Run, arrivals: &Opened) -> Leases {
    Leases::new(run.clock(), run.id().as_str(), run.state().epoch())
        .staging_in(arrivals.staging.clone())
}

/// The delivery's loop, which returns once the Run ended or stopped.
async fn deliver(
    run: &Run,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    settings: &RunSettings,
    fetching: &Fetching<'_>,
) {
    let run_leases = leases_of(run, arrivals);
    let Some((mut admission, mut pool)) = opened(run, settings).await else {
        return;
    };
    let mut wakes = admission.wakes();
    let mut attempts: JoinSet<Attempt> = JoinSet::new();
    let mut stop: Option<StopReason> = None;
    loop {
        let resolved = settings.at_admission().await;
        let policy = CrawlPolicy::of(&resolved);
        admission.update(&resolved);
        if stop.is_none()
            && let Err(error) = lease_to_servers(
                run,
                &run_leases,
                policy,
                Pooled::of(&mut pool, &resolved, fetching),
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
                    match drained(run, &resolved, &mut admission, servers, arrivals).await {
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
        let waits = [stop.is_some(), free, starved];
        show(run, &admission, &resolved, servers, waits);
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
            Some(ended) = attempts.join_next() => {
                if let Ok(attempt) = ended {
                    let pool = (&mut pool, fetching);
                    settled(run, settings, &mut admission, pool, servers, attempt).await;
                }
            }
            () = sleep_until(run.clock(), wake), if wake.is_some() => {}
            () = run.governor().turn(WorkKind::Http), if free && starved => {}
            key = next_wake(wakes.as_mut()) => admission.wake(key.as_deref()),
        }
    }
}

/// Sets the crawl's live parts of its Run's progress (ADR 0102) as its
/// `servers` stand, given whether it stops, whether a serving Connection
/// has room and whether the governor starves it.
fn show(
    run: &Run,
    admission: &Admission,
    resolved: &Resolved,
    servers: &mut [Serving],
    [stopping, free, starved]: [bool; 3],
) {
    let serving = servers.iter_mut().any(Serving::serves);
    let handlers = servers.iter().map(|server| server.in_flight).sum();
    let room = progress::room(stopping || !serving, free, starved);
    let concurrency = CrawlPolicy::of(resolved).handler_concurrency;
    progress::show(run, admission, resolved, room, (handlers, concurrency));
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
    let policy = CrawlPolicy::of(&settings.at_admission().await);
    let retry_max = policy.retry_max;
    let bounds = Bounds::of_settings(settings.resolved());
    let resolved = settings.resolved().clone();
    let left = run
        .state()
        .write(move |connection| {
            bounds.record(connection)?;
            goodfetch_frontier::payload::record_max_size(connection, &resolved)?;
            work_items::recover_leases(connection, HANDLER_LOST.name, retry_max)?;
            Ok((
                work_items::left(connection)?,
                labels::read(connection)?,
                labels::Fragments::read(connection)?,
                tier_choice::read(connection)?,
                code_free::read(connection)?,
            ))
        })
        .await;
    let left = left.map(|(left, declared, fragments, choices, extraction)| {
        let labels = labels::Declared::of(&declared, fragments).unwrap_or_else(|error| {
            tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's Labels no longer read; its requests take the Run's tier and pool");
            labels::Declared::default()
        });
        (left, (labels, choices, extraction))
    });
    let (labels, choices, extraction) = match left {
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
    let opened = crawls.open(&run, egress).await;
    let code_free = CodeFree::of(extraction, &crawls, &tiers);
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
    labels: &'a labels::Declared,
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
    /// How the navigation gate fetches robots.txt.
    robots: Option<RobotsTxtFetch>,
    /// What a code-free crawl handles its requests with; none when its
    /// program does.
    code_free: Option<std::sync::Arc<CodeFree>>,
    /// The AI-use signal files the Run fetched, each once.
    signal_files: SignalFiles,
    /// The Run's domain policy, which judges each redirect hop.
    egress: std::sync::Arc<goodfetch_engine::egress::Policy>,
    /// The requests in flight, as the Run's progress shows them.
    in_flight: SharedInFlight,
}

impl Fetching<'_> {
    /// What one attempt's handler pages, of a request of `label`, are
    /// gated and judged with.
    fn gating(&self, label: &str, second_copies: &SecondCopies) -> Gating {
        Gating {
            tiers: self.tiers.clone(),
            second_copies: second_copies.clone(),
            robots: self.robots.clone(),
            egress: std::sync::Arc::clone(&self.egress),
            addresses: self.http.map(HttpTier::address_check),
            page_check: self.labels.settings.page_check(label).cloned(),
            encoding: self
                .labels
                .settings
                .encoding(label, self.run_encoding.as_deref()),
            content_types: self
                .labels
                .settings
                .content_types(label, &self.run_content_types),
            code_free: self.code_free.clone(),
            signal_files: self.signal_files.clone(),
        }
    }

    /// How `leased`'s attempt runs, as its Label and the Run's
    /// `fetch.tier` choose and, under `auto`, on the browser for a domain a
    /// Challenge moved there, `to_browser` (ADR 0021), else, when the Run
    /// races its tiers, as a race timed by its Label on its domain's
    /// browser-fetch latencies, else as its Label on its domain learned,
    /// and what it fetches its document with.
    async fn served(
        &self,
        leased: &Leased,
        (to_browser, local): (bool, Local),
        (second_copies, now): (&SecondCopies, u64),
    ) -> Tiering {
        let request = &leased.crawl_request;
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
        let plan = match choosing {
            Choosing::Alone | Choosing::Engine if keeps_fragment => AttemptPlan::On(TierChoice {
                tier: FetchTier::Browser,
                forced: None,
            }),
            Choosing::Alone | Choosing::Engine if self.races && !to_browser && !leased.sitemap => {
                let learned = self.choices.lock().await.serve_attempt(
                    &request.label,
                    &leased.waiting.key,
                    &request.url,
                    now,
                );
                AttemptPlan::Race(Raced {
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
                })
            }
            Choosing::Alone => AttemptPlan::Alone,
            Choosing::Forced(choice) => AttemptPlan::On(choice),
            Choosing::Engine if to_browser => AttemptPlan::On(TierChoice {
                tier: FetchTier::Browser,
                forced: None,
            }),
            Choosing::Engine => self.choices.lock().await.serve_attempt(
                &request.label,
                &leased.waiting.key,
                &request.url,
                now,
            ),
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

    /// Classifies the sample `attempt` was, if it was one, and writes what
    /// its Label on its domain now learned into the run file.
    async fn classify(&self, run: &Run, attempt: &mut Attempt) {
        let Some((sampled, weighed)) = attempt.sampled.take() else {
            return;
        };
        let unit = self
            .choices
            .lock()
            .await
            .classify(&sampled, weighed, wall_ms(run))
            .clone();
        if let Err(error) = run
            .state()
            .write(move |connection| {
                tier_choice::record(connection, &sampled.label, &sampled.domain_key, &unit)
            })
            .await
        {
            tracing::warn!(%error, run_id = %run.id(), "a Label's tier choice was not written; a resumed crawl samples it again");
        }
    }
}

/// What each attempt is given: a Session drawn from the crawl's Session
/// pool under the Run's settings now, and the tier that fetches its
/// request's document.
struct Pooled<'a> {
    pool: &'a mut SessionPool,
    settings: PoolSettings,
    fetching: &'a Fetching<'a>,
}

impl<'a> Pooled<'a> {
    /// Draws from `pool` under `resolved`, fetching on `fetching`.
    fn of(pool: &'a mut SessionPool, resolved: &Resolved, fetching: &'a Fetching<'a>) -> Self {
        Self {
            pool,
            settings: PoolSettings::of(resolved),
            fetching,
        }
    }
}

/// Admits work items up to each serving Connection's room and the HTTP
/// admissions the engine's Resource governor gives the Run now, draws the
/// Session of the pool that serves each, written in run state before any
/// handler request is sent, and starts an attempt for each on that
/// Connection, which holds its admission until it ends (ADR 0040). A Run
/// the governor gave none waits for its turn.
async fn lease_to_servers(
    run: &Run,
    run_leases: &Leases,
    policy: CrawlPolicy,
    pooled: Pooled<'_>,
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
        let labels = &pooled.fetching.labels.pools;
        let served: Vec<(Leased, ServingSession, PooledSession, (bool, Local))> =
            admit(run, admission, permits.len())
                .await?
                .into_iter()
                .map(|leased| {
                    let key = labels.pool(&leased.crawl_request.label);
                    let session = pooled.pool.draw(
                        &key,
                        &leased.waiting.key,
                        &pooled.settings,
                        run.clock().wall(),
                    );
                    let in_browser = PooledSession {
                        id: session.id.clone(),
                        source: labels.source(&key),
                    };
                    let to_browser = pooled.pool.browser(&leased.waiting.key);
                    let local = pooled.pool.local_addresses(&session.id);
                    (leased, session, in_browser, (to_browser, local))
                })
                .collect();
        for rotated in pooled.pool.rotated_within_block() {
            warn_rotated(run, rotated).await;
        }
        pooled
            .fetching
            .retire(run, pooled.pool, &server.context)
            .await;
        let changes = pooled.pool.changes();
        if !changes.is_empty() {
            run.state()
                .write(move |connection| changes.write(connection))
                .await?;
        }
        for (leased, session, in_browser, (to_browser, local)) in served {
            let lease = run_leases.grant(
                &leased.crawl_request.id,
                leased.lease_epoch,
                policy.lease_length(),
            );
            server.in_flight += 1;
            let tiering = pooled
                .fetching
                .served(
                    &leased,
                    (to_browser, local),
                    (&admission.second_copies(), wall_ms(run)),
                )
                .await;
            let gating = pooled
                .fetching
                .gating(&leased.crawl_request.label, &admission.second_copies());
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
            let permit = permits
                .pop()
                .expect("invariant: one admission per work item admitted");
            attempts.spawn(holding(
                permit,
                attempt(
                    run.clone(),
                    place,
                    server.context.clone(),
                    (leased, (session, in_browser), leased_count(run)),
                    lease,
                    (tiering, gating),
                    std::sync::Arc::clone(&pooled.fetching.in_flight),
                ),
            ));
        }
    }
    Ok(())
}

/// Runs `attempt`, holding `permit` until it ends.
async fn holding(permit: Permit, attempt: impl Future<Output = Attempt>) -> Attempt {
    let ended = attempt.await;
    drop(permit);
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
/// cut as its Completeness reasons, unless `crawl.keep_alive`,
/// read at each admission point as its policy is, holds it until
/// `run.finish` lets it finish: until then it waits for new work, as the
/// Run's own timeout and a stop still end it (RT-12).
async fn drained(
    run: &Run,
    resolved: &Resolved,
    admission: &mut Admission,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
) -> Waited {
    let keeps_alive = resolved
        .boolean(CRAWL_KEEP_ALIVE.name)
        .expect("invariant: `crawl.keep_alive` resolves to a boolean");
    if !keeps_alive || run.is_let_finish() {
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

/// Settles an attempt that ended under the Run's policy now, takes it out
/// of the requests in flight its progress shows, frees its place on its
/// Connection, returns its domain's turn, puts a retried work item back in
/// its queue and journals what the queues changed.
async fn settled(
    run: &Run,
    settings: &RunSettings,
    admission: &mut Admission,
    (pool, fetching): (&mut SessionPool, &Fetching<'_>),
    servers: &mut [Serving],
    mut attempt: Attempt,
) {
    fetching.classify(run, &mut attempt).await;
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
    let merged = std::mem::take(&mut attempt.merged);
    let resolved = settings.at_admission().await;
    let domain = domain_ending(
        &attempt.answer,
        attempt.document.as_ref(),
        proxied(&resolved),
        run.clock().now(),
        run.clock().wall(),
    );
    pool.settle(
        &attempt.session,
        &waiting.key,
        effect(&attempt.answer),
        &PoolSettings::of(&resolved),
    );
    let changes = pool.changes();
    if !changes.is_empty()
        && let Err(error) = run
            .state()
            .write(move |connection| changes.write(connection))
            .await
    {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's Session pool was not written");
    }
    fetching
        .retire(run, pool, &servers[attempt.server].context)
        .await;
    let retried = match attempt.answer {
        Answer::SitemapRead(_) => {
            read_sitemap(run, &resolved, attempt).await;
            false
        }
        _ => {
            settle(
                run,
                CrawlPolicy::of(&resolved),
                goodfetch_frontier::longest_wait(&resolved),
                attempt,
            )
            .await
        }
    };
    admission
        .ended(&waiting.id, &waiting.key, run.clock().now(), domain)
        .await;
    if retried {
        count_retry(run);
        admission.enqueue(waiting);
    }
    let mut keys = BTreeSet::new();
    for (id, key) in merged {
        admission.withdraw(&key, &id);
        keys.insert(key);
    }
    for key in keys {
        if let Err(error) = fill(run, admission, Some(key)).await {
            tracing::warn!(%error, run_id = %run.id(), "a queue a learned rule thinned was not filled again");
        }
    }
    if let Err(error) = key_new(run, admission).await {
        tracing::warn!(%error, run_id = %run.id(), "the requests a handler enqueued were not keyed yet");
    }
    journal(run, admission).await;
}

/// Opens the Run's admission and Session pool, or ends the Run as failed
/// when they could not open.
async fn opened(run: &Run, settings: &RunSettings) -> Option<(Admission, SessionPool)> {
    match open(run, settings).await {
        Ok(opened) => Some(opened),
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
    let resolved = settings.at_admission().await;
    let mut admission = run
        .admission(&resolved)
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
    let pool = match Rotation::of(&resolved) {
        Some(rotation) => pool.with_local_addresses(LocalAddressing::new(
            rotation,
            BlockSizes::of(&resolved),
            std::sync::Arc::new(System),
            proxied(&resolved),
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
    match LocalAddresses::parse(resolved.value(EGRESS_LOCAL_ADDRESSES.name)) {
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

/// Whether the Run's Sessions send through a proxy, whose exit rotates in
/// place of their local address (ADRs 0021 and 0044).
fn proxied(resolved: &Resolved) -> bool {
    [PROXY_SERVER.name, PROXY_PROVIDER.name]
        .iter()
        .any(|key| !resolved.value(key).is_null())
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
    run.state()
        .write(move |connection| work_items::set_keys(connection, &keys))
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

/// Admits up to `room` work items ready now and leases them, then fills
/// again each domain's queue that drained. Each one first charges the
/// request it is against the Limits that hold the Run (ADR 0041): once one
/// has nothing left, or one the engine cannot evaluate covers it, the Run
/// is asked to stop at its next safe point and nothing more is admitted.
async fn admit(
    run: &Run,
    admission: &mut Admission,
    room: usize,
) -> Result<Vec<Leased>, ChargeError> {
    let now = run.clock().now();
    let mut ids = Vec::new();
    let mut keys = Vec::new();
    let mut drained = Vec::new();
    let mut refused = Vec::new();
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
        match run.charge(REQUESTS, 1).await {
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
    let leased = if ids.is_empty() {
        Vec::new()
    } else {
        run.state()
            .write(move |connection| work_items::lease(connection, &ids))
            .await?
    };
    // A work item a rule learned later merged after the gate took it is
    // no longer pending: it gives its domain's turn back unsent.
    for (id, key) in asked.iter().zip(&keys) {
        if !leased.iter().any(|leased| leased.crawl_request.id == *id) {
            admission.ended(id, key, now, Ended::Done).await;
        }
    }
    for key in drained {
        fill(run, admission, Some(key)).await?;
    }
    Ok(leased)
}

/// Writes what the gate changed into the run file.
async fn journal(run: &Run, admission: &mut Admission) {
    let clock = run.clock();
    if let Some(journal) = admission.journal(clock.wall(), clock.now())
        && let Err(error) = run
            .state()
            .write(move |connection| journal(connection))
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

/// Commits an attempt's outcome: its Records once while its lease holds,
/// its handler's skip, its block, or its work item retried or failed; then
/// releases its lease. Answers whether its work item waits again for a
/// retry.
async fn settle(
    run: &Run,
    policy: CrawlPolicy,
    longest_wait: Option<Duration>,
    attempt: Attempt,
) -> bool {
    let Attempt {
        leased,
        lease,
        answer,
        ..
    } = attempt;
    let failure = |failure, retryable| retried_or_failed(policy, leased.tries, failure, retryable);
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
        // Read by `read_sitemap`, never settled here.
        Answer::SitemapRead(_) => {
            lease.release();
            return false;
        }
        Answer::Handled(handled) => {
            match lease
                .commit(
                    run.state(),
                    run.secrets().mask().masks(),
                    handled.records,
                    run.point_in_time(),
                )
                .await
            {
                Ok(files) => {
                    lease.release();
                    goodfetch_engine::key_value_store::forget_files(run, files).await;
                    return false;
                }
                Err(CommitError::LeaseLost) => failure(
                    Failure::new(LEASE_LOST.name, LEASE_LOST.description),
                    LEASE_LOST.retryable,
                ),
                Err(CommitError::Refused(refused)) => failure(
                    kept_failure(
                        run,
                        refused.code().to_owned(),
                        refused.message(),
                        refused.details().cloned().unwrap_or_default(),
                    ),
                    refused.retryable(),
                ),
                Err(CommitError::RunState(error)) => {
                    tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                    return false;
                }
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
            let held = held_past_bound(&details, run.clock().wall(), longest_wait).is_some();
            let failed = kept_failure(run, code, &message, details);
            match block_cause(&failed.code, &failed.details) {
                Some(cause) => Unsuccessful::Blocked(cause),
                None if held => Unsuccessful::Failed(failed.with_fix(RETRY_AFTER_FIX)),
                None => failure(failed, retryable),
            }
        }
    };
    end_unsuccessful(run, &lease, &leased, unsuccessful).await
}

/// Reads what the sitemap fetch `attempt` gave while its lease holds, in
/// one write: its entries and nested sitemaps admitted under the Run's
/// domain policy as `resolved` holds it, within the scope the run file
/// holds, and the fetch ended with no outcome (ADR 0093); the delivery
/// then keys the work items it added. Releases its lease.
async fn read_sitemap(run: &Run, resolved: &Resolved, attempt: Attempt) {
    let Attempt {
        leased,
        lease,
        answer: Answer::SitemapRead(document),
        ..
    } = attempt
    else {
        return;
    };
    let egress = resumed_domain_policy(resolved, run.id());
    let (id, epoch) = (leased.crawl_request.id.clone(), leased.lease_epoch);
    let written = run
        .state()
        .write(move |connection| {
            work_items::read_sitemap(connection, &egress, (&id, epoch), &document)
        })
        .await;
    lease.release();
    match written {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "a sitemap's fetch past its lease reads nothing");
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "a sitemap's entries were not admitted");
        }
    }
}

/// A handler's failure as the Run's state keeps it: no value the Run
/// resolved stays in its message or details (spec #564).
fn kept_failure(run: &Run, code: String, message: &str, details: Map<String, Value>) -> Failure {
    let masks = run.secrets().mask().masks();
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

/// Ends `leased`'s work item `unsuccessful` while its lease holds,
/// counting its failure's code in the Run's outcomes, then releases
/// `lease`. Answers whether the work item waits again for a retry.
async fn end_unsuccessful(
    run: &Run,
    lease: &WorkItemLease,
    leased: &Leased,
    unsuccessful: Unsuccessful,
) -> bool {
    let id = leased.crawl_request.id.clone();
    let epoch = leased.lease_epoch;
    let retried = matches!(unsuccessful, Unsuccessful::Retried(_));
    let recorded = match &unsuccessful {
        Unsuccessful::Retried(failure) | Unsuccessful::Failed(failure) => {
            Some(failure.code.clone())
        }
        Unsuccessful::Skipped { .. } | Unsuccessful::Blocked(_) => None,
    };
    let ended_at = run.point_in_time();
    let written = run
        .state()
        .write(move |connection| work_items::end(connection, &id, epoch, unsuccessful, &ended_at))
        .await;
    lease.release();
    match written {
        Ok(true) => {
            if let Some(code) = recorded {
                run.record_outcome(&code);
            }
            retried
        }
        Ok(false) => {
            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "an attempt past its lease commits nothing");
            false
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
            false
        }
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
