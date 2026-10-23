//! A crawl's per-domain Politeness through the engine's rule interface
//! (spec #213's Seam 4), two runs' rules under their own settings on one
//! Politeness file's table, at wall readings the tests give: each run's own
//! cap against the count in flight together, the delay since any sharing
//! run's last start, a `Retry-After`, a backoff and a 429's halving met by
//! one run holding every other, and the ceiling growing back.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::politeness::{Ending, PermitRule, Verdict};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{DomainRule, TABLES};

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
    DomainRule::of(&resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap())
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

/// Covers: story:3-80
#[rstest]
#[case::retry_after(Ending::Until(at(30)), at(30))]
#[case::backoff(Ending::BackOff, at(2))]
#[case::too_many_requests(Ending::TooManyRequests(None), at(2))]
#[case::too_many_requests_with_retry_after(Ending::TooManyRequests(Some(at(45))), at(45))]
fn what_one_run_met_holds_every_run_sharing_the_key(
    #[case] ending: Ending,
    #[case] until: SystemTime,
) {
    let file = file();
    let ready_at = unlimited().ended(&file, KEY, 0, ending, at(0)).unwrap();

    let other = concurrency(8).admit(&file, KEY, 0, at(1)).unwrap();

    assert_eq!((ready_at, other), (Some(until), Verdict::Until(until)));
}

/// Covers: story:3-80
#[test]
fn backoffs_in_a_row_double_and_a_success_resets_them() {
    let file = file();
    let first = unlimited()
        .ended(&file, KEY, 0, Ending::BackOff, at(0))
        .unwrap();
    let second = unlimited()
        .ended(&file, KEY, 0, Ending::BackOff, at(2))
        .unwrap();
    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(6))
        .unwrap();
    let after_success = unlimited()
        .ended(&file, KEY, 0, Ending::BackOff, at(7))
        .unwrap();

    assert_eq!(
        (first, second, after_success),
        (Some(at(2)), Some(at(6)), Some(at(9)))
    );
}

/// A 429 met with four requests in flight halves the key's concurrency to
/// two for every sharing run, even one with no cap of its own, and a quiet
/// ten seconds later each success raises it by one.
///
/// Covers: story:3-80
#[test]
fn a_429_halves_the_keys_concurrency_for_every_run_and_it_grows_back() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 3, Ending::TooManyRequests(None), at(0))
        .unwrap();

    let halved = (
        unlimited().admit(&file, KEY, 1, at(2)).unwrap(),
        unlimited().admit(&file, KEY, 2, at(2)).unwrap(),
    );
    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(5))
        .unwrap();
    let still = unlimited().admit(&file, KEY, 2, at(6)).unwrap();
    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(10))
        .unwrap();
    let grown = unlimited().admit(&file, KEY, 2, at(11)).unwrap();

    assert_eq!(
        (halved, still, grown),
        ((Verdict::Go, Verdict::Busy), Verdict::Busy, Verdict::Go)
    );
}
