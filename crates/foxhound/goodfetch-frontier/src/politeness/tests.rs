//! A crawl's per-domain Politeness through the engine's rule interface
//! (spec #213's Seam 4), two runs' rules under their own settings on one
//! Politeness file's table, at wall readings the tests give: each run's own
//! cap against the count in flight together, the delay since any sharing
//! run's last start, a `Retry-After`, a backoff and a 429's halving met by
//! one run holding every other, and the ceiling growing back; a
//! `Retry-After` past a run's `politeness.retry_after.max` refusing its
//! requests unsent; and the backoff's waits, drawn with decorrelated jitter
//! from a seeded `Entropy` (ADR 0118); and a host's Crawl-delay folded
//! into its domain key's delay up to `site_preferences.max_crawl_delay`,
//! for every run sharing the key (ADR 0086).

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::{Ending, Hold, KeyState, Verdict};
use goodfetch_engine::settings::{Layers, Resolved, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{
    Backoff, DocumentReads, DocumentStart, DomainRule, DomainState, Pace, Parts, TABLES, Wrote,
};
use crate::robots_txt;

mod pacing;

/// A rule's steps on a Politeness file's connection, each loading the
/// key's state and saving what it wrote as a Permit's take and give-back
/// do.
mod on_file {
    use std::time::SystemTime;

    use goodfetch_engine::politeness::{Ending, PermitRule, Step, Verdict};
    use rusqlite::Connection;

    use super::{DomainRule, DomainState, Wrote, changed};

    pub(super) trait OnFile {
        fn admit(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            now: SystemTime,
        ) -> rusqlite::Result<Verdict>;

        fn started(&self, file: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()>;

        fn ended(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            ending: Ending,
            now: SystemTime,
        ) -> rusqlite::Result<Option<SystemTime>>;
    }

    impl OnFile for DomainRule {
        fn admit(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            now: SystemTime,
        ) -> rusqlite::Result<Verdict> {
            changed(
                file,
                key,
                PermitRule::reads(self, Step::Take),
                |state: &mut DomainState| {
                    (
                        PermitRule::admit(self, state, in_flight, now),
                        Wrote::default(),
                    )
                },
            )
        }

        fn started(&self, file: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()> {
            changed(
                file,
                key,
                PermitRule::reads(self, Step::Take),
                |state: &mut DomainState| ((), PermitRule::started(self, state, now)),
            )
        }

        fn ended(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            ending: Ending,
            now: SystemTime,
        ) -> rusqlite::Result<Option<SystemTime>> {
            changed(
                file,
                key,
                PermitRule::reads(self, Step::GiveBack),
                |state: &mut DomainState| PermitRule::ended(self, state, in_flight, ending, now),
            )
        }
    }
}

use on_file::OnFile as _;

/// Loads the state kept under `key` in `file` as `reads` says, runs `change`
/// on it and saves what it wrote, as one `Politeness::change` does inside
/// its transaction.
fn changed<S: KeyState, T>(
    file: &Connection,
    key: &S::Key,
    reads: S::Reads,
    change: impl FnOnce(&mut S) -> (T, S::Writes),
) -> rusqlite::Result<T> {
    let mut state = S::load(file, key, reads)?;
    let (answer, writes) = change(&mut state);
    state.save(file, key, &writes)?;
    Ok(answer)
}

/// Records in `file` that the robots.txt at `location` asks the requests
/// of `key` for the Crawl-delay `delay`, or for none.
fn crawl_delay_asked(
    file: &Connection,
    key: &str,
    location: &str,
    delay: Option<Duration>,
) -> rusqlite::Result<()> {
    changed(file, key, Parts::CRAWL_DELAYS, |state: &mut DomainState| {
        ((), state.crawl_delay_asked(location.to_owned(), delay))
    })
}

/// Takes in `file` the next start of `key` for a document after the
/// robots.txt at `location` ([`DocumentStart::taken`]).
fn document_start_taken(
    file: &Connection,
    key: &str,
    location: &str,
    fetched: Option<SystemTime>,
    spacing: Duration,
    now: SystemTime,
) -> rusqlite::Result<Option<SystemTime>> {
    let reads = DocumentReads {
        fetched,
        spacing,
        now,
    };
    changed(
        file,
        &(key.to_owned(), location.to_owned()),
        reads,
        |state: &mut DocumentStart| state.taken(reads),
    )
}

/// Takes in `file` the start of a request of `key` sent at `now`
/// ([`DomainState::send_taken`]).
fn send_taken(
    file: &Connection,
    key: &str,
    now: SystemTime,
    pace: Option<Pace>,
) -> rusqlite::Result<Option<SystemTime>> {
    changed(
        file,
        key,
        DomainState::send_reads(pace.as_ref()),
        |state: &mut DomainState| state.send_taken(now, pace),
    )
}

/// Records in `file` a response of `key` that arrived `latency` after its
/// request was sent ([`DomainState::latency_noted`]).
fn latency_noted(file: &Connection, key: &str, latency: Duration) -> rusqlite::Result<()> {
    changed(file, key, Parts::PACING, |state: &mut DomainState| {
        ((), state.latency_noted(latency))
    })
}

/// Moves in `file` the token a request of `key` sent at `sent` took
/// ([`DomainState::start_moved`]).
fn start_moved(file: &Connection, key: &str, sent: SystemTime) -> rusqlite::Result<()> {
    changed(file, key, Parts::PACING, |state: &mut DomainState| {
        ((), state.start_moved(sent))
    })
}

const KEY: &str = "shop.test";

fn file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES
        .statements
        .iter()
        .chain(robots_txt::TABLES.statements)
    {
        connection.execute(statement, []).unwrap();
    }
    connection
}

/// Keeps in `file` a copy of the robots.txt at `location` fetched at
/// `fetched_at`.
fn copy_fetched(file: &Connection, location: &str, fetched_at: SystemTime) {
    let copy = robots_txt::Copy {
        fetched_at: goodfetch_engine::politeness::milliseconds(fetched_at),
        status: 200,
        body: b"User-agent: *\nCrawl-delay: 5\n".to_vec(),
    };
    robots_txt::store(file, location, &copy).unwrap();
}

/// The settings of a run whose own layer is `run`.
fn resolved(run: Value) -> Resolved {
    let Value::Object(run) = run else {
        panic!("settings are a table");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap()
}

fn rule(run: Value) -> DomainRule {
    DomainRule::of(&resolved(run), Arc::new(entropy(7).into()))
}

/// The pace a run whose own layer is `run` sends at.
fn pace(run: Value) -> Pace {
    Pace::of(&resolved(run))
}

fn concurrency(cap: u64) -> DomainRule {
    rule(json!({ "politeness": { "domain": { "concurrency": cap } } }))
}

fn unlimited() -> DomainRule {
    rule(json!({}))
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(491_001) + Duration::from_secs(seconds)
}

/// Covers: story:3-81, setting:politeness.domain.concurrency
#[test]
fn a_stricter_run_waits_while_a_looser_one_goes_up_to_its_own_cap() {
    let file = file();

    let verdicts = (
        concurrency(1).admit(&file, KEY, 1, at(0)).unwrap(),
        concurrency(2).admit(&file, KEY, 1, at(0)).unwrap(),
        concurrency(2).admit(&file, KEY, 2, at(0)).unwrap(),
    );

    assert_eq!(verdicts, (Verdict::Busy, Verdict::Go, Verdict::Busy));
}

/// Covers: story:3-80, setting:politeness.domain.delay
#[test]
fn each_run_waits_its_own_delay_after_any_sharing_runs_last_start() {
    let file = file();
    let one_second = rule(json!({ "politeness": { "domain": { "delay": 1_000 } } }));
    let two_seconds = rule(json!({ "politeness": { "domain": { "delay": 2_000 } } }));
    one_second.started(&file, KEY, at(10)).unwrap();

    let verdicts = (
        one_second.admit(&file, KEY, 0, at(11)).unwrap(),
        two_seconds.admit(&file, KEY, 0, at(11)).unwrap(),
    );

    assert_eq!(verdicts, (Verdict::Go, Verdict::Until(at(12), Hold::Delay)));
}

/// Covers: story:3-80, story:3-159
#[rstest]
#[case::retry_after(Ending::Until(at(30)), (at(30), at(30)))]
#[case::backoff(Ending::BackOff(None), (at(2), at(6)))]
#[case::backoff_with_retry_after(Ending::BackOff(Some(at(30))), (at(30), at(30)))]
#[case::too_many_requests(Ending::TooManyRequests(None), (at(2), at(6)))]
#[case::too_many_requests_with_retry_after(Ending::TooManyRequests(Some(at(45))), (at(45), at(45)))]
fn what_one_run_met_holds_every_run_sharing_the_key(
    #[case] ending: Ending,
    #[case] (earliest, latest): (SystemTime, SystemTime),
) {
    let file = file();
    let ready_at = unlimited()
        .ended(&file, KEY, 0, ending, at(0))
        .unwrap()
        .expect("the key waits");

    let other = concurrency(8).admit(&file, KEY, 0, at(1)).unwrap();

    assert!(
        (earliest..=latest).contains(&ready_at),
        "{ready_at:?} outside {earliest:?}..={latest:?}"
    );
    assert_eq!(other, Verdict::Until(ready_at, Hold::BackOff));
}

/// A `Retry-After` shorter than the backoff it came with adds no wait: the
/// domain waits the longer of the two (ADR 0118).
///
/// Covers: story:3-159
#[test]
fn a_retry_after_shorter_than_the_backoff_leaves_the_backoff() {
    let file = file();

    let ready_at = unlimited()
        .ended(&file, KEY, 0, Ending::BackOff(Some(at(1))), at(0))
        .unwrap()
        .expect("the key waits");

    assert!((at(2)..=at(6)).contains(&ready_at), "{ready_at:?}");
}

/// Backoffs in a row grow from 2 s toward 60 s, and a success resets them:
/// the wait after it is drawn as a first one again.
///
/// Covers: story:3-80, story:3-158
#[test]
fn backoffs_in_a_row_grow_toward_a_minute_and_a_success_resets_them() {
    let file = file();
    let rule = unlimited();
    let mut now = at(0);
    let mut waits = Vec::new();
    for _ in 0..40 {
        let ready_at = rule
            .ended(&file, KEY, 0, Ending::BackOff(None), now)
            .unwrap()
            .expect("the key waits");
        waits.push(ready_at.duration_since(now).unwrap());
        now = ready_at;
    }
    rule.ended(&file, KEY, 0, Ending::Done, now).unwrap();
    let after_success = rule
        .ended(&file, KEY, 0, Ending::BackOff(None), now)
        .unwrap()
        .expect("the key waits")
        .duration_since(now)
        .unwrap();

    assert!(
        waits
            .iter()
            .all(|wait| *wait >= Duration::from_secs(2) && *wait <= Duration::from_mins(1)),
        "{waits:?}"
    );
    assert!(
        waits.iter().any(|wait| *wait > Duration::from_secs(30)),
        "{waits:?}"
    );
    assert!(after_success <= Duration::from_secs(6), "{after_success:?}");
}

/// A run's `politeness.backoff.initial` sets the shortest wait its
/// backoff draws, the first spread up to three times it.
///
/// Covers: story:3-158, setting:politeness.backoff.initial
#[test]
fn a_runs_initial_wait_bounds_its_first_backoff() {
    let file = file();
    let patient = rule(json!({ "politeness": { "backoff": { "initial": 10_000 } } }));

    let ready_at = patient
        .ended(&file, KEY, 0, Ending::BackOff(None), at(0))
        .unwrap()
        .expect("the key waits");

    assert!((at(10)..=at(30)).contains(&ready_at), "{ready_at:?}");
}

/// A `Retry-After` of ten minutes holds every sharing run past
/// `politeness.backoff.max`, within `politeness.retry_after.max`.
///
/// Covers: story:3-159, setting:politeness.backoff.max
#[test]
fn a_retry_after_past_the_backoffs_longest_wait_holds_every_run_until_then() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 0, Ending::BackOff(Some(at(600))), at(0))
        .unwrap();

    let other = concurrency(8).admit(&file, KEY, 0, at(1)).unwrap();

    assert_eq!(other, Verdict::Until(at(600), Hold::BackOff));
}

/// A site that asks for an hour is past the 15 minutes a run waits by
/// default: its requests are refused unsent until the site's time is within
/// the bound, then wait for it; a run whose bound is longer waits at once
/// (ADR 0118).
///
/// Covers: story:3-160, setting:politeness.retry_after.max
#[test]
fn a_retry_after_past_a_runs_bound_refuses_its_requests_until_the_time_is_within_it() {
    let file = file();
    let patient = rule(json!({ "politeness": { "retry_after": { "max": 7_200_000 } } }));
    unlimited()
        .ended(
            &file,
            KEY,
            0,
            Ending::TooManyRequests(Some(at(3_600))),
            at(0),
        )
        .unwrap();

    let verdicts = (
        unlimited().admit(&file, KEY, 0, at(1)).unwrap(),
        unlimited().admit(&file, KEY, 0, at(2_701)).unwrap(),
        patient.admit(&file, KEY, 0, at(1)).unwrap(),
    );

    assert_eq!(
        verdicts,
        (
            Verdict::Refused(at(3_600)),
            Verdict::Until(at(3_600), Hold::BackOff),
            Verdict::Until(at(3_600), Hold::BackOff)
        )
    );
}

/// A run with `politeness.backoff.enabled` off ignores the backoffs and the
/// `Retry-After` any run met, and adds none of its own (ADRs 0023 and 0118).
///
/// Covers: story:3-159, setting:politeness.backoff.enabled
#[test]
fn a_run_with_backoff_off_ignores_backoffs_and_retry_after() {
    let (file, untouched) = (file(), file());
    let off = rule(json!({ "politeness": { "backoff": { "enabled": false } } }));
    unlimited()
        .ended(&file, KEY, 0, Ending::BackOff(Some(at(30))), at(0))
        .unwrap();

    let goes = off.admit(&file, KEY, 0, at(1)).unwrap();
    let added = off
        .ended(
            &untouched,
            KEY,
            0,
            Ending::TooManyRequests(Some(at(30))),
            at(0),
        )
        .unwrap();

    assert_eq!((goes, added), (Verdict::Go, None));
}

proptest! {
    /// Whatever a site asked and whatever a run's bound, the domain admits
    /// no request before the time the site asked (ADR 0118's G5).
    ///
    /// Covers: story:3-159, story:3-160
    #[test]
    fn nothing_is_admitted_before_the_time_the_site_asked(
        asked in 1_u64..20_000,
        bound in 0_u64..20_000_000,
        earlier in 1_u64..20_000,
    ) {
        let file = file();
        let run = rule(json!({ "politeness": { "retry_after": { "max": bound } } }));
        run.ended(&file, KEY, 0, Ending::BackOff(Some(at(asked))), at(0)).unwrap();

        let verdict = run.admit(&file, KEY, 0, at(asked.saturating_sub(earlier))).unwrap();

        prop_assert_ne!(verdict, Verdict::Go);
    }
}

/// A 429 met with four requests in flight halves the key's concurrency to
/// two for every sharing run, even one with no cap of its own, once its
/// backoff (at most 6 s) ended, and a quiet ten seconds later each success
/// raises it by one.
///
/// Covers: story:3-80
#[test]
fn a_429_halves_the_keys_concurrency_for_every_run_and_it_grows_back() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 3, Ending::TooManyRequests(None), at(0))
        .unwrap();

    let halved = (
        unlimited().admit(&file, KEY, 1, at(7)).unwrap(),
        unlimited().admit(&file, KEY, 2, at(7)).unwrap(),
    );
    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(8))
        .unwrap();
    let still = unlimited().admit(&file, KEY, 2, at(9)).unwrap();
    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(10))
        .unwrap();
    let grown = unlimited().admit(&file, KEY, 2, at(11)).unwrap();

    assert_eq!(
        (halved, still, grown),
        ((Verdict::Go, Verdict::Busy), Verdict::Busy, Verdict::Go)
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "the Entropy's test version, seeded by the case (ADR 0059)"
)]
fn entropy(seed: u64) -> Entropy {
    Entropy::seeded(seed)
}

fn milliseconds(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// The robots.txt locations of two hosts of [`KEY`].
const SHOP: &str = "https://shop.test/robots.txt";
const WWW: &str = "https://www.shop.test/robots.txt";

/// A Crawl-delay one run read raises the delay between starts of every run
/// sharing the key, the largest of its hosts' applying; one at most the
/// run's `politeness.domain.delay` does not shorten it; one above its
/// `site_preferences.max_crawl_delay` does not raise it; with
/// `site_preferences.respect_robots_txt` off none applies; and a host that
/// stops asking no longer raises it.
///
/// Covers: story:6-243, setting:site_preferences.max_crawl_delay, setting:politeness.domain.delay
#[rstest]
#[case::raises_the_delay(json!({}), &[(SHOP, Some(5))], at(15))]
#[case::the_largest_host_applies(json!({}), &[(SHOP, Some(2)), (WWW, Some(4))], at(14))]
#[case::below_the_delay_never_shortens_it(
    json!({ "politeness": { "domain": { "delay": 3_000 } } }),
    &[(SHOP, Some(2))],
    at(13)
)]
#[case::above_the_bound_is_not_waited(
    json!({ "site_preferences": { "max_crawl_delay": 3_000 } }),
    &[(SHOP, Some(5)), (WWW, Some(2))],
    at(12)
)]
/// Covers: setting:site_preferences.respect_robots_txt
#[case::not_with_robots_txt_off(
    json!({ "site_preferences": { "respect_robots_txt": false } }),
    &[(SHOP, Some(5))],
    at(11)
)]
#[case::a_host_that_stops_asking(json!({}), &[(SHOP, Some(5)), (SHOP, None)], at(11))]
#[case::with_no_delay_of_its_own(
    json!({ "politeness": { "domain": { "delay": 0 } } }),
    &[(SHOP, Some(5))],
    at(15)
)]
fn a_hosts_crawl_delay_is_folded_into_its_keys_delay_for_every_sharing_run(
    #[case] run: Value,
    #[case] asked: &[(&str, Option<u64>)],
    #[case] next: SystemTime,
) {
    let file = file();
    for (location, seconds) in asked {
        crawl_delay_asked(&file, KEY, location, seconds.map(Duration::from_secs)).unwrap();
    }
    unlimited().started(&file, KEY, at(10)).unwrap();

    let verdicts = (
        rule(run.clone()).admit(&file, KEY, 0, at(11)).unwrap(),
        rule(run).admit(&file, KEY, 0, next).unwrap(),
    );

    let first = if next == at(11) {
        Verdict::Go
    } else {
        Verdict::Until(next, Hold::Delay)
    };
    assert_eq!(verdicts, (first, Verdict::Go));
}

/// Two runs that each fetched the key's robots.txt at once, before either
/// knew its Crawl-delay, take their documents' starts one Crawl-delay
/// apart, the second after the first's, and a third run's admission waits
/// a Crawl-delay after the later.
///
/// Covers: story:6-243
#[test]
fn documents_after_robots_txt_fetched_at_once_start_a_delay_apart() {
    let file = file();
    let crawl_delay = Duration::from_secs(5);
    crawl_delay_asked(&file, KEY, SHOP, Some(crawl_delay)).unwrap();
    unlimited().started(&file, KEY, at(10)).unwrap();

    let now = at(10) + milliseconds(10);
    let starts = (
        document_start_taken(&file, KEY, SHOP, Some(at(10)), crawl_delay, now).unwrap(),
        document_start_taken(
            &file,
            KEY,
            SHOP,
            Some(at(10) + milliseconds(5)),
            crawl_delay,
            now,
        )
        .unwrap(),
    );
    let third = unlimited().admit(&file, KEY, 0, at(16)).unwrap();

    assert_eq!(
        (starts, third),
        (
            (Some(at(15)), Some(at(20))),
            Verdict::Until(at(25), Hold::Delay)
        )
    );
}

/// A document's start after a robots.txt that arrived between two
/// milliseconds is never sooner than the Crawl-delay after it: the
/// Politeness file keeps whole milliseconds, so the start is the next one.
///
/// Covers: story:6-243
#[test]
fn a_start_after_a_moment_inside_a_millisecond_is_never_sooner_than_the_delay_after_it() {
    let file = file();
    let crawl_delay = Duration::from_secs(5);
    let arrived = at(10) + Duration::from_micros(400);

    let start =
        document_start_taken(&file, KEY, SHOP, Some(arrived), crawl_delay, arrived).unwrap();

    assert_eq!(start, Some(at(15) + milliseconds(1)));
}

/// A run admitted before the Crawl-delay was known, which reads the copy
/// another run fetched less than the Crawl-delay before instead of
/// fetching robots.txt itself, starts its document the Crawl-delay after
/// that fetch, and the run that fetched it the Crawl-delay after that.
/// (The copy keeps whole milliseconds, so the fetch counts from the next.)
///
/// Covers: story:6-243
#[test]
fn a_document_after_a_copy_another_run_fetched_within_the_delay_starts_the_delay_after_that_fetch()
{
    let file = file();
    let crawl_delay = Duration::from_secs(5);
    unlimited().started(&file, KEY, at(10)).unwrap();
    unlimited()
        .started(&file, KEY, at(10) + milliseconds(1))
        .unwrap();
    copy_fetched(&file, SHOP, at(10) + milliseconds(8));
    crawl_delay_asked(&file, KEY, SHOP, Some(crawl_delay)).unwrap();
    let now = at(10) + milliseconds(20);

    let starts = (
        document_start_taken(&file, KEY, SHOP, None, crawl_delay, now).unwrap(),
        document_start_taken(
            &file,
            KEY,
            SHOP,
            Some(at(10) + milliseconds(8)),
            crawl_delay,
            now,
        )
        .unwrap(),
    );

    assert_eq!(
        starts,
        (
            Some(at(15) + milliseconds(9)),
            Some(at(20) + milliseconds(9))
        )
    );
}

/// A document after a copy fetched at least the delay before takes no
/// start of its own: its admission already kept the delay.
///
/// Covers: story:6-243
#[test]
fn a_document_after_a_copy_fetched_a_delay_or_more_before_takes_no_start() {
    let file = file();
    let crawl_delay = Duration::from_secs(5);
    copy_fetched(&file, SHOP, at(0));
    crawl_delay_asked(&file, KEY, SHOP, Some(crawl_delay)).unwrap();
    unlimited().started(&file, KEY, at(10)).unwrap();

    let start = document_start_taken(&file, KEY, SHOP, None, crawl_delay, at(10)).unwrap();
    let next = unlimited().admit(&file, KEY, 0, at(11)).unwrap();

    assert_eq!((start, next), (None, Verdict::Until(at(15), Hold::Delay)));
}

proptest! {
    /// Whatever the run's delay, its bound and the Crawl-delays its key's
    /// hosts ask for, the key admits a request at a wall reading exactly
    /// when its last start is the run's delay ago and each Crawl-delay
    /// within the bound ago, and until then holds it to that moment: one
    /// above the bound never holds it.
    ///
    /// Covers: story:6-243, setting:site_preferences.max_crawl_delay
    #[test]
    fn the_key_is_held_until_its_delay_and_every_crawl_delay_within_the_bound_passed(
        delay in 0_u64..10_000,
        bound in 0_u64..60_000,
        crawl_delays in proptest::collection::vec(0_u64..90_000, 0..4),
        after in 0_u64..100_000,
    ) {
        let file = file();
        for (host, asked) in crawl_delays.iter().enumerate() {
            crawl_delay_asked(
                &file,
                KEY,
                &format!("https://{host}.shop.test/robots.txt"),
                Some(milliseconds(*asked)),
            )
            .unwrap();
        }
        let run = rule(json!({
            "politeness": { "domain": { "delay": delay } },
            "site_preferences": { "max_crawl_delay": bound },
        }));
        run.started(&file, KEY, at(10)).unwrap();
        let ready = at(10)
            + milliseconds(
                crawl_delays
                    .iter()
                    .copied()
                    .filter(|asked| *asked <= bound)
                    .fold(delay, u64::max),
            );
        let now = at(10) + milliseconds(after);

        let verdict = run.admit(&file, KEY, 0, now).unwrap();

        if now >= ready {
            prop_assert_eq!(verdict, Verdict::Go);
        } else {
            prop_assert_eq!(verdict, Verdict::Until(ready, Hold::Delay));
        }
    }
}

proptest! {
    /// Each wait of a domain's backoff in a row is drawn between
    /// `politeness.backoff.initial` and three times the previous wait, and
    /// none is above `politeness.backoff.max` (ADR 0118).
    ///
    /// Covers: story:3-158, setting:politeness.backoff.initial, setting:politeness.backoff.max
    #[test]
    fn each_wait_stays_within_initial_max_and_three_times_the_previous(
        seed: u64,
        initial in 1_u64..10_000,
        longer in 0_u64..200_000,
        backoffs in 1_usize..24,
    ) {
        let (initial, max) = (milliseconds(initial), milliseconds(initial + longer));
        let backoff = Backoff { initial, max };
        let mut entropy = entropy(seed);
        let mut previous: Option<Duration> = None;

        for _ in 0..backoffs {
            let wait = backoff.next(previous, &mut entropy);

            prop_assert!(wait >= initial && wait <= max, "{wait:?} outside {initial:?}..={max:?}");
            prop_assert!(wait <= previous.unwrap_or(initial) * 3, "{wait:?} after {previous:?}");
            previous = Some(wait);
        }
    }

    /// Domains behind one origin that back off together draw their first
    /// waits apart, spread over 2 s to 6 s, so they do not come back
    /// together (ADR 0118).
    ///
    /// Covers: story:3-158
    #[test]
    fn domains_backing_off_together_draw_first_waits_spread_over_initial_to_three_times_it(
        seed: u64,
    ) {
        let backoff = Backoff { initial: Duration::from_secs(2), max: Duration::from_mins(1) };
        let mut entropy = entropy(seed);

        let waits: Vec<Duration> = (0..32).map(|_| backoff.next(None, &mut entropy)).collect();

        prop_assert!(waits.iter().all(|wait| (Duration::from_secs(2)..=Duration::from_secs(6)).contains(wait)), "{waits:?}");
        prop_assert!(waits.iter().any(|wait| *wait < Duration::from_secs(4)), "{waits:?}");
        prop_assert!(waits.iter().any(|wait| *wait > Duration::from_secs(4)), "{waits:?}");
    }
}

/// A rule's steps on the tables a Run's own Politeness holds in memory
/// (`politeness.sharing: run`, #2171) answer what they answer on a
/// Politeness file: the same caps, delays, backoff, `Retry-After` and 429
/// halving (spec #2138's story 31).
mod held_in_memory {
    use std::time::SystemTime;

    use goodfetch_engine::politeness::{
        Ending, Hold, KeyState, MemoryTables, PermitRule, Step, Verdict,
    };
    use rusqlite::Connection;
    use serde_json::json;

    use super::{
        DomainRule, DomainState, Parts, Wrote, at, changed, concurrency, file, rule, unlimited,
    };

    /// Where a rule's steps load a key's state and save what they wrote.
    trait Tables {
        fn step<T>(
            &mut self,
            key: &str,
            reads: Parts,
            change: impl FnOnce(&mut DomainState) -> (T, Wrote),
        ) -> T;
    }

    impl Tables for Connection {
        fn step<T>(
            &mut self,
            key: &str,
            reads: Parts,
            change: impl FnOnce(&mut DomainState) -> (T, Wrote),
        ) -> T {
            changed(self, key, reads, change).unwrap()
        }
    }

    impl Tables for MemoryTables {
        fn step<T>(
            &mut self,
            key: &str,
            reads: Parts,
            change: impl FnOnce(&mut DomainState) -> (T, Wrote),
        ) -> T {
            let mut state = DomainState::load_from(self, key, reads);
            let (answer, writes) = change(&mut state);
            state.save_to(self, key, &writes);
            answer
        }
    }

    /// What a step answered: a take's verdict, or when a give back lets
    /// its key go again.
    #[derive(Clone, Debug, PartialEq)]
    enum Answered {
        Verdict(Verdict),
        ReadyAt(Option<SystemTime>),
    }

    fn admit(
        tables: &mut impl Tables,
        rule: &DomainRule,
        (key, in_flight): (&str, u64),
        now: SystemTime,
    ) -> Answered {
        Answered::Verdict(tables.step(key, rule.reads(Step::Take), |state| {
            (rule.admit(state, in_flight, now), Wrote::default())
        }))
    }

    fn started(tables: &mut impl Tables, rule: &DomainRule, key: &str, now: SystemTime) {
        tables.step(key, rule.reads(Step::Take), |state| {
            ((), rule.started(state, now))
        });
    }

    fn ended(
        tables: &mut impl Tables,
        rule: &DomainRule,
        (key, in_flight): (&str, u64),
        ending: Ending,
        now: SystemTime,
    ) -> Answered {
        Answered::ReadyAt(tables.step(key, rule.reads(Step::GiveBack), |state| {
            rule.ended(state, in_flight, ending, now)
        }))
    }

    /// A key's cap, a delay since its last start, a backoff, a
    /// `Retry-After` holding a stricter run, and a 429's halving growing
    /// back, each on a key of its own, step by step.
    fn steps(tables: &mut impl Tables) -> Vec<Answered> {
        let (capped, free) = (concurrency(2), unlimited());
        let delayed = rule(json!({ "politeness": { "domain": { "delay": 1_000 } } }));
        let mut answered = vec![
            admit(tables, &capped, ("cap.test", 1), at(0)),
            admit(tables, &capped, ("cap.test", 2), at(0)),
        ];
        started(tables, &delayed, "delay.test", at(0));
        answered.push(admit(tables, &delayed, ("delay.test", 0), at(0)));
        answered.push(admit(tables, &delayed, ("delay.test", 0), at(1)));
        answered.push(ended(
            tables,
            &free,
            ("backoff.test", 0),
            Ending::BackOff(None),
            at(0),
        ));
        answered.push(admit(tables, &capped, ("backoff.test", 0), at(1)));
        ended(
            tables,
            &free,
            ("asked.test", 0),
            Ending::BackOff(Some(at(600))),
            at(0),
        );
        answered.push(admit(tables, &capped, ("asked.test", 0), at(1)));
        ended(
            tables,
            &free,
            ("halving.test", 3),
            Ending::TooManyRequests(None),
            at(0),
        );
        answered.push(admit(tables, &free, ("halving.test", 1), at(7)));
        answered.push(admit(tables, &free, ("halving.test", 2), at(7)));
        ended(tables, &free, ("halving.test", 0), Ending::Done, at(8));
        ended(tables, &free, ("halving.test", 0), Ending::Done, at(10));
        answered.push(admit(tables, &free, ("halving.test", 2), at(11)));
        answered
    }

    /// Covers: story:3-81, story:3-159, setting:politeness.sharing
    #[test]
    fn a_runs_own_politeness_keeps_the_caps_delays_backoff_and_429_halving_of_the_file() {
        let on_file = steps(&mut file());
        let in_memory = steps(&mut MemoryTables::default());

        assert_eq!(in_memory, on_file);
        assert_eq!(
            [&in_memory[..4], &in_memory[6..]].concat(),
            [
                Answered::Verdict(Verdict::Go),
                Answered::Verdict(Verdict::Busy),
                Answered::Verdict(Verdict::Until(at(1), Hold::Delay)),
                Answered::Verdict(Verdict::Go),
                Answered::Verdict(Verdict::Until(at(600), Hold::BackOff)),
                Answered::Verdict(Verdict::Go),
                Answered::Verdict(Verdict::Busy),
                Answered::Verdict(Verdict::Go),
            ]
        );
    }
}
