//! What a crawl adds to its Run's progress (ADR 0102). Its delivery sets
//! the live parts as they change: `waiting`, why its due requests wait, one
//! group per reason with the domains it holds and the setting behind it;
//! `inFlight`, each attempt with its phase; `idle`, while a keep-alive crawl
//! waits for work; and `concurrency`. Its composition's progress part reads
//! `requests`, and `sitemaps`, each sitemap seed's sitemaps read and failed
//! and its entries' counts, from committed state each time a value is
//! read, and keeps in
//! `inFlight` and `concurrency` only the attempts that state leases, so a
//! value is one moment of the crawl. A Run of another kind gets none of
//! them.

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_engine::admission::{Held, QueuedGroup};
use goodfetch_engine::runs::{ProgressPart, Run};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::CRAWL_HANDLER_CONCURRENCY;
use goodfetch_protocol::foxhound::progress::{
    BACKOFF, CrawlConcurrency, DOMAIN_CONCURRENCY, DOMAIN_DELAY, GOVERNOR, HANDLER_CONCURRENCY,
    HELD_DOMAINS, HeldDomain, InFlightPhase, InFlightRequest, POLITENESS_FILE, RequestProgress,
    WaitGroup,
};
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_BACKOFF_MAX, POLITENESS_DOMAIN_CONCURRENCY, POLITENESS_DOMAIN_DELAY,
    POLITENESS_RETRY_AFTER_MAX,
};
use goodfetch_protocol::run::{RunKind, RunProgress};
use goodfetch_protocol::settings::SettingLimit;
use rusqlite::Connection;
use serde_json::Value;
use tokio::time::Instant;

use crate::work_items;

/// The names of a crawl's parts in its Run's progress.
const WAITING: &str = "waiting";
const IN_FLIGHT: &str = "in_flight";
const IDLE: &str = "idle";
const CONCURRENCY: &str = "concurrency";
const REQUESTS: &str = "requests";
const SITEMAPS: &str = "sitemaps";

/// The part `$part` encoded as its JSON value.
macro_rules! encoded {
    ($part:expr) => {
        serde_json::to_value($part).expect("invariant: a progress part encodes")
    };
}

/// Whether the crawl's due requests that no domain holds may go now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Room {
    /// A serving Connection has room and the governor admits.
    Free,
    /// Every serving Connection runs `crawl.handler.concurrency` handlers.
    Full,
    /// The Resource governor admits nothing new (ADR 0040).
    Starved,
    /// Nothing goes: the Run stops, or no Connection serves it.
    None,
}

/// Whether the due requests no domain holds may go: none while the Run
/// stops or no Connection serves it (`held`), else as the serving
/// Connections' room (`free`) and the governor (`starved`) say.
pub(crate) fn room(held: bool, free: bool, starved: bool) -> Room {
    if held {
        Room::None
    } else if !free {
        Room::Full
    } else if starved {
        Room::Starved
    } else {
        Room::Free
    }
}

/// Sets the crawl's live parts of its Run's progress: why its due requests
/// wait, `queued` in its admission at `now`, under `resolved`, with `room`
/// for those no domain holds; its handlers running against
/// `crawl.handler.concurrency`; and that it is not idle.
pub(crate) fn show(
    run: &Run,
    (queued, now): (&[QueuedGroup], Instant),
    resolved: &Resolved,
    room: Room,
    (handlers, handler_concurrency): (usize, usize),
) {
    show_waits(run, queued, resolved, now, room);
    show_concurrency(run, handlers, handler_concurrency);
    show_idle(run, false);
}

/// Sets the crawl's `waiting` from the groups its admission queues at
/// `now`, each with at most [`HELD_DOMAINS`] domains, under `resolved`,
/// with `room` for the requests no domain holds: a few groups, whatever
/// the number of domains.
pub(crate) fn show_waits(
    run: &Run,
    queued: &[QueuedGroup],
    resolved: &Resolved,
    now: Instant,
    room: Room,
) {
    let mut groups: BTreeMap<&'static str, WaitGroup> = BTreeMap::new();
    for queued in queued {
        let first_until = queued.sample.first().and_then(|(_, until)| *until);
        let (reason, setting) = match queued.held {
            Some(Held::Concurrency) => (
                DOMAIN_CONCURRENCY,
                setting(resolved, POLITENESS_DOMAIN_CONCURRENCY.name),
            ),
            Some(Held::Delay) => (
                DOMAIN_DELAY,
                setting(resolved, POLITENESS_DOMAIN_DELAY.name),
            ),
            Some(Held::BackOff) => (BACKOFF, backoff_setting(resolved, first_until, now)),
            Some(Held::PolitenessFile) => (POLITENESS_FILE, None),
            None => match room {
                Room::Full => (
                    HANDLER_CONCURRENCY,
                    setting(resolved, CRAWL_HANDLER_CONCURRENCY.name),
                ),
                Room::Starved => (GOVERNOR, None),
                Room::Free | Room::None => continue,
            },
        };
        let shown: Vec<HeldDomain> = queued
            .sample
            .iter()
            .take(HELD_DOMAINS)
            .map(|(domain, until)| HeldDomain {
                domain: domain.clone(),
                next_admitted_at: until
                    .map(|until| run.point_in_time_after(until.saturating_duration_since(now))),
            })
            .collect();
        let more = queued.keys.saturating_sub(shown.len());
        groups.insert(
            reason,
            WaitGroup {
                reason: reason.to_owned(),
                count: u64::try_from(queued.requests).unwrap_or(u64::MAX),
                domains: shown,
                more_domains: u64::try_from(more).unwrap_or(u64::MAX),
                setting,
            },
        );
    }
    let waiting: Vec<WaitGroup> = groups.into_values().collect();
    run.set_progress_part(WAITING, encoded!(&waiting));
}

/// The setting `key` as `resolved` gives it, with the layer that set it.
fn setting(resolved: &Resolved, key: &str) -> Option<SettingLimit> {
    let effective = resolved.get(key)?;
    Some(SettingLimit {
        key: key.to_owned(),
        value: effective.value.clone(),
        layer: effective.layer,
    })
}

/// The setting behind a backoff that holds its domain until `until`:
/// `politeness.retry_after.max` when the wait is longer than any backoff,
/// so a site's `Retry-After` set it, else `politeness.backoff.max` (ADR
/// 0118).
fn backoff_setting(
    resolved: &Resolved,
    until: Option<Instant>,
    now: Instant,
) -> Option<SettingLimit> {
    let longest_backoff = resolved
        .duration(POLITENESS_BACKOFF_MAX.name)
        .unwrap_or(Duration::MAX);
    let waits = until.map_or(Duration::ZERO, |until| until.saturating_duration_since(now));
    if waits > longest_backoff {
        setting(resolved, POLITENESS_RETRY_AFTER_MAX.name)
    } else {
        setting(resolved, POLITENESS_BACKOFF_MAX.name)
    }
}

/// Sets the crawl's `idle`: whether `crawl.keep_alive` holds it with no
/// work.
pub(crate) fn show_idle(run: &Run, idle: bool) {
    run.set_progress_part(IDLE, Value::Bool(idle));
}

/// Sets the crawl's `concurrency`: `handlers` running against
/// `handler_concurrency`, and what the Resource governor last read.
pub(crate) fn show_concurrency(run: &Run, handlers: usize, handler_concurrency: usize) {
    let info = run.system_info();
    let concurrency = CrawlConcurrency {
        handlers: u64::try_from(handlers).unwrap_or(u64::MAX),
        handler_concurrency: u64::try_from(handler_concurrency).unwrap_or(u64::MAX),
        governor: info.as_ref().map(|info| info.level),
        open_pages: info
            .as_ref()
            .map(|info| info.concurrency.browser_page.in_use),
        page_allowance: info
            .as_ref()
            .map(|info| info.concurrency.browser_page.target),
        allowance_set_by: info.as_ref().map(|info| info.memory.allowance_source),
    };
    run.set_progress_part(CONCURRENCY, encoded!(&concurrency));
}

/// The crawl's requests in flight, each as its progress shows it, by work
/// item ID: its delivery adds and removes each, and each attempt says when
/// it goes on from fetching to handling.
#[derive(Debug, Default)]
pub(crate) struct InFlight {
    requests: BTreeMap<String, InFlightRequest>,
    /// How many waits for a navigation gate's permit each request's
    /// handler pages have under way.
    permit_waits: BTreeMap<String, usize>,
}

/// The crawl's requests in flight, shared by its delivery and its
/// attempts; an async lock, never held across another await.
pub(crate) type SharedInFlight = std::sync::Arc<tokio::sync::Mutex<InFlight>>;

impl InFlight {
    /// `request` began.
    pub(crate) fn began(&mut self, run: &Run, request: InFlightRequest) {
        self.requests.insert(request.request_id.clone(), request);
        self.show(run);
    }

    /// The request `id` went on to `phase`.
    pub(crate) fn went_on(&mut self, run: &Run, id: &str, phase: InFlightPhase) {
        if let Some(request) = self.requests.get_mut(id) {
            request.phase = phase;
        }
        self.show(run);
    }

    /// A page of the request `id`'s handler began to wait for a navigation
    /// gate's permit: the request waits for one until every such wait ended.
    pub(crate) fn permit_wait_began(&mut self, run: &Run, id: &str) {
        *self.permit_waits.entry(id.to_owned()).or_default() += 1;
        self.went_on(run, id, InFlightPhase::WaitingForPermit);
    }

    /// A wait [`InFlight::permit_wait_began`] counted ended.
    pub(crate) fn permit_wait_ended(&mut self, run: &Run, id: &str) {
        let left = self.permit_waits.get_mut(id).map(|waits| {
            *waits = waits.saturating_sub(1);
            *waits
        });
        if left == Some(0) {
            self.permit_waits.remove(id);
            self.went_on(run, id, InFlightPhase::Handling);
        }
    }

    /// The request `id` ended.
    pub(crate) fn ended(&mut self, run: &Run, id: &str) {
        self.requests.remove(id);
        self.permit_waits.remove(id);
        self.show(run);
    }

    fn show(&self, run: &Run) {
        let requests: Vec<&InFlightRequest> = self.requests.values().collect();
        run.set_progress_part(IN_FLIGHT, encoded!(&requests));
    }
}

/// The crawl's part of its Run's progress read from committed state:
/// `requests`, its requests by state, and which are in flight. The live
/// parts were taken at another moment than the committed state, and the
/// delivery commits an attempt's lease and its outcome apart from adding
/// it to `inFlight` and taking it out with its handler, so the committed
/// state decides which requests are in flight: `inFlight` keeps those it leases at the
/// same attempt, split by phase in `requests`, and `concurrency` counts
/// their handlers; one it leases that `inFlight` does not show yet is
/// counted pending, as it was a moment before. One read is then one
/// moment of the crawl.
pub(crate) struct CrawlProgress;

impl ProgressPart for CrawlProgress {
    fn fill(&self, connection: &Connection, progress: &mut RunProgress) -> rusqlite::Result<()> {
        let kind: String = connection
            .prepare_cached("SELECT kind FROM run")
            .and_then(|mut cached| cached.query_row([], |row| row.get(0)))?;
        if kind != RunKind::Crawl.as_str() {
            return Ok(());
        }
        let (ended, _) = crate::report::ended(connection)?;
        let (pending, leased) = work_items::pending_and_leased(connection)?;
        let in_flight: Vec<InFlightRequest> = progress
            .kind_parts
            .get(IN_FLIGHT)
            .and_then(|requests| {
                serde_json::from_value::<Vec<InFlightRequest>>(requests.clone()).ok()
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|request| leased.contains(&(request.request_id.clone(), request.attempt)))
            .collect();
        let count = |requests: usize| u64::try_from(requests).unwrap_or(u64::MAX);
        let fetching = in_flight
            .iter()
            .filter(|request| request.phase == InFlightPhase::Fetching)
            .count();
        let starting = leased.len() - in_flight.len();
        let handlers = in_flight.len();
        if let Some(concurrency) = progress.kind_parts.get_mut(CONCURRENCY) {
            concurrency["handlers"] = Value::from(count(handlers));
        }
        progress
            .kind_parts
            .insert(IN_FLIGHT.to_owned(), encoded!(&in_flight));
        let requests = RequestProgress {
            pending: pending + count(starting),
            fetching: count(fetching),
            handling: count(handlers - fetching),
            in_human_pause: 0,
            succeeded: ended.succeeded,
            failed: ended.failed,
            skipped: ended.skipped,
            blocked: ended.blocked,
            by_skip_reason: ended.by_skip_reason,
            by_error_code: ended.by_error_code,
            by_block_cause: ended.by_block_cause,
        };
        progress
            .kind_parts
            .insert(REQUESTS.to_owned(), encoded!(&requests));
        let sitemaps = goodfetch_frontier::sitemap::reports(connection)?;
        if !sitemaps.is_empty() {
            progress
                .kind_parts
                .insert(SITEMAPS.to_owned(), encoded!(&sitemaps));
        }
        Ok(())
    }
}
