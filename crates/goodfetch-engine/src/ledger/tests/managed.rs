//! Managed rows (ADR 0041): the install Limits a managed Operator policy
//! names, written by each engine of its major when it starts as rows of
//! their own, which only the policy and `upgrade-files` change.
//!
//! Covers: story:3-77

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::usage::{LimitUsage, ManagedLimit};
use goodfetch_test_support::TestEnv;

use super::{charging, install, ledger, limit, now};
use crate::ledger::Ledger;

/// The policy's install Limit of `amount` requests.
fn requests(amount: u64) -> ManagedLimit {
    ManagedLimit {
        usage_kind: "requests".to_owned(),
        currency: None,
        period: None,
        amount,
    }
}

/// Each row as `usage.list` shows it: who set it, its managed major, its
/// amount and the Usage charged against it.
fn rows(ledger: &Ledger) -> Vec<(LimitSetter, Option<String>, u64, u64)> {
    ledger
        .usage(Product::Collie, now())
        .unwrap()
        .into_iter()
        .map(|usage: LimitUsage| {
            (
                usage.limit.set_by,
                usage.managed_major,
                usage.limit.amount,
                usage.charged,
            )
        })
        .collect()
}

fn managed(major: &str, amount: u64, charged: u64) -> (LimitSetter, Option<String>, u64, u64) {
    (
        LimitSetter::ManagedPolicy,
        Some(major.to_owned()),
        amount,
        charged,
    )
}

#[test]
fn an_engine_writes_the_install_limit_of_its_policy_as_a_managed_row_of_its_major() {
    let env = TestEnv::new();
    let ledger = ledger(&env);

    ledger
        .write_managed("2", &[requests(100).limit()], now())
        .unwrap();

    assert_eq!(rows(&ledger), [managed("2", 100, 0)]);
}

#[test]
fn a_changed_policy_changes_its_row_in_place_keeping_the_usage_counted() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .write_managed("2", &[requests(100).limit()], now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 3, now()).unwrap();
    run.give_back().unwrap();

    ledger
        .write_managed("2", &[requests(50).limit()], now())
        .unwrap();

    assert_eq!(rows(&ledger), [managed("2", 50, 3)]);
}

#[test]
fn a_limit_the_policy_no_longer_names_is_dropped_and_other_rows_stay() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10), now())
        .unwrap();
    ledger
        .write_managed("1", &[requests(70).limit()], now())
        .unwrap();
    ledger
        .write_managed("2", &[requests(100).limit()], now())
        .unwrap();

    ledger.write_managed("2", &[], now()).unwrap();

    assert_eq!(
        rows(&ledger),
        [(LimitSetter::Person, None, 10, 0), managed("1", 70, 0)]
    );
}

#[test]
fn a_person_raising_the_install_limit_leaves_the_managed_row_as_it_is() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .write_managed("2", &[requests(100).limit()], now())
        .unwrap();

    ledger
        .raise(&limit(install(), "requests", 1_000), now())
        .unwrap();

    assert_eq!(
        rows(&ledger),
        [managed("2", 100, 0), (LimitSetter::Person, None, 1_000, 0)]
    );
}

/// Whether `major` is older than major 3.
fn older_than_three(major: &str) -> bool {
    major.parse::<u64>().is_ok_and(|major| major < 3)
}

#[test]
fn upgrade_files_gives_the_new_major_the_older_ones_rows_and_drops_the_rest() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .write_managed("1", &[requests(10).limit()], now())
        .unwrap();
    ledger
        .write_managed("2", &[requests(100).limit()], now())
        .unwrap();
    ledger
        .write_managed("4", &[requests(400).limit()], now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 3, now()).unwrap();
    run.give_back().unwrap();

    let counts = ledger.upgrade_managed("2", "3", older_than_three).unwrap();

    assert_eq!(
        (counts, rows(&ledger)),
        ((1, 1), vec![managed("3", 100, 3), managed("4", 400, 3)])
    );
}

#[test]
fn upgrade_files_keeps_the_new_majors_own_rows_over_the_older_ones() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .write_managed("2", &[requests(100).limit()], now())
        .unwrap();
    ledger
        .write_managed("3", &[requests(300).limit()], now())
        .unwrap();

    let counts = ledger.upgrade_managed("2", "3", older_than_three).unwrap();

    assert_eq!(
        (counts, rows(&ledger)),
        ((0, 1), vec![managed("3", 300, 0)])
    );
}

#[test]
fn an_engine_whose_policy_names_no_install_limit_leaves_an_uncreated_ledger_uncreated() {
    let env = TestEnv::new();
    let ledger = ledger(&env);

    ledger.write_managed("2", &[], now()).unwrap();

    assert!(!ledger.path().exists());
}
