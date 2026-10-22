//! Reserved costs (ADR 0041): a model call's, paid solve's or Paid
//! fetch's worst case reserved against a currency Limit before it is sent,
//! taking enough Slice in one transaction or refused, then settled at what
//! it cost.
//!
//! Covers: story:3-70

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::ReachedLimit;
use goodfetch_protocol::usage::Limit;
use goodfetch_test_support::TestEnv;

use super::{charging, install, ledger, limit, now};
use crate::ledger::{Ledger, Refusal};

/// A Limit of 1 USD, in millionths, on paid solves.
fn dollar() -> Limit {
    Limit {
        currency: Some("USD".to_owned()),
        ..limit(install(), "paid_solves", 1_000_000)
    }
}

/// The first Limit's charged, held and left Usage now.
fn counted(ledger: &Ledger) -> (u64, u64, u64) {
    let usage = ledger.usage(Product::Collie, now()).unwrap();
    (usage[0].charged, usage[0].held, usage[0].left)
}

#[test]
fn a_worst_case_above_the_slice_takes_enough_in_one_slice() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.add_limit(&dollar(), now()).unwrap();
    let mut run = charging(&env, "solver");

    let (_, charged) = run
        .reserve("paid_solves", Some("USD"), 50_000, now())
        .unwrap();

    assert_eq!(
        (charged.used.len(), counted(&ledger)),
        (1, (0, 50_000, 950_000))
    );
}

#[test]
fn a_worst_case_beyond_what_is_left_is_refused_and_takes_nothing() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.add_limit(&dollar(), now()).unwrap();
    let mut run = charging(&env, "solver");
    let _open = run
        .reserve("paid_solves", Some("USD"), 600_000, now())
        .unwrap();

    let refused = run.reserve("paid_solves", Some("USD"), 600_000, now());

    assert_eq!(
        (refused.map(drop), counted(&ledger)),
        (
            Err(Refusal::Reached(Box::new(ReachedLimit {
                scope: install(),
                usage_kind: "paid_solves".to_owned(),
                currency: Some("USD".to_owned()),
                period: None,
                resets_at: None,
            }))),
            (0, 600_000, 400_000)
        )
    );
}

#[test]
fn a_settled_reservation_charges_what_it_cost_when_the_run_ends() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.add_limit(&dollar(), now()).unwrap();
    let mut run = charging(&env, "solver");
    let (reservation, _) = run
        .reserve("paid_solves", Some("USD"), 50_000, now())
        .unwrap();

    run.settle(reservation, 30_000);
    run.give_back().unwrap();

    assert_eq!(counted(&ledger), (30_000, 0, 970_000));
}

#[test]
fn a_reservation_still_open_when_the_run_ends_stays_charged_at_its_worst_case() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.add_limit(&dollar(), now()).unwrap();
    let mut run = charging(&env, "solver");
    let _open = run
        .reserve("paid_solves", Some("USD"), 50_000, now())
        .unwrap();

    run.give_back().unwrap();

    assert_eq!(counted(&ledger), (50_000, 0, 950_000));
}

#[test]
fn a_reservation_never_counts_against_a_limit_in_another_currency() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.add_limit(&dollar(), now()).unwrap();
    let mut run = charging(&env, "solver");

    let _open = run
        .reserve("paid_solves", Some("EUR"), 50_000, now())
        .unwrap();

    assert_eq!(counted(&ledger), (0, 0, 1_000_000));
}
