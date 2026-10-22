//! A crawl's per-domain Politeness through the engine's rule interface
//! (spec #213's Seam 4), two runs' rules under their own settings on one
//! Politeness file's table, at wall readings the tests give: each run's own
//! cap against the count in flight together, the delay since any sharing
//! run's last start, a `Retry-After`, a backoff and a 429's halving met by
//! one run holding every other, and the ceiling growing back; a
//! `Retry-After` past a run's `politeness.retry_after.max` refusing its
//! requests unsent; and the backoff's waits, drawn with decorrelated jitter
//! from a seeded `Entropy` (ADR 0118).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::{Ending, PermitRule, Verdict};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Backoff, DomainRule, TABLES};

const KEY: &str = "shop.test";

fn file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
}

fn rule(run: Value) -> DomainRule {
    let Value::Object(run) = run else {
        panic!("settings are a table");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    DomainRule::of(
        &resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap(),
        Arc::new(entropy(7).into()),
    )
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

    assert_eq!(verdicts, (Verdict::Go, Verdict::Until(at(12))));
}

/// Covers: story:3-80, story:3-154
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
    assert_eq!(other, Verdict::Until(ready_at));
}

/// A `Retry-After` shorter than the backoff it came with adds no wait: the
/// domain waits the longer of the two (ADR 0118).
///
/// Covers: story:3-154
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
/// Covers: story:3-80, story:3-153
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
/// Covers: story:3-153, setting:politeness.backoff.initial
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
/// Covers: story:3-154, setting:politeness.backoff.max
#[test]
fn a_retry_after_past_the_backoffs_longest_wait_holds_every_run_until_then() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 0, Ending::BackOff(Some(at(600))), at(0))
        .unwrap();

    let other = concurrency(8).admit(&file, KEY, 0, at(1)).unwrap();

    assert_eq!(other, Verdict::Until(at(600)));
}

/// A site that asks for an hour is past the 15 minutes a run waits by
/// default: its requests are refused unsent until the site's time is within
/// the bound, then wait for it; a run whose bound is longer waits at once
/// (ADR 0118).
///
/// Covers: story:3-155, setting:politeness.retry_after.max
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
            Verdict::Until(at(3_600)),
            Verdict::Until(at(3_600))
        )
    );
}

/// A run with `politeness.backoff.enabled` off ignores the backoffs and the
/// `Retry-After` any run met, and adds none of its own (ADRs 0023 and 0118).
///
/// Covers: story:3-154, setting:politeness.backoff.enabled
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
    /// Covers: story:3-154, story:3-155
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

proptest! {
    /// Each wait of a domain's backoff in a row is drawn between
    /// `politeness.backoff.initial` and three times the previous wait, and
    /// none is above `politeness.backoff.max` (ADR 0118).
    ///
    /// Covers: story:3-153, setting:politeness.backoff.initial, setting:politeness.backoff.max
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
    /// Covers: story:3-153
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
