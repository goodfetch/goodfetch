//! Who allows paying (ADRs 0041 and 0045): the ledger tells a run who set
//! the Limit that applies to a currency it would pay in. Only a Limit a
//! person or a managed Operator policy set allows paying; a Client's own
//! Limit, which may lower a person's beside it, caps payments but never
//! allows them, and run and Schedule Limits are no ledger rows.
//!
//! Covers: story:3-75

use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::usage::Limit;
use goodfetch_test_support::TestEnv;
use rstest::rstest;

use super::{charging, install, ledger, limit, now};

/// An install Limit on paid solves in `currency` that `set_by` set.
fn solves(currency: &str, set_by: LimitSetter, amount: u64) -> Limit {
    Limit {
        currency: Some(currency.to_owned()),
        set_by,
        ..limit(install(), "paid_solves", amount)
    }
}

#[rstest]
#[case::no_limit(&[], None)]
#[case::a_clients_own(&[(LimitSetter::Client, 5)], None)]
#[case::a_persons(&[(LimitSetter::Person, 10)], Some(LimitSetter::Person))]
#[case::a_managed_policy(&[(LimitSetter::ManagedPolicy, 10)], Some(LimitSetter::ManagedPolicy))]
#[case::a_persons_a_client_lowered(
    &[(LimitSetter::Person, 10), (LimitSetter::Client, 4)],
    Some(LimitSetter::Person),
)]
fn only_a_limit_a_person_or_a_managed_policy_set_allows_paying(
    #[case] set: &[(LimitSetter, u64)],
    #[case] allowed_by: Option<LimitSetter>,
) {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    for (set_by, amount) in set {
        ledger
            .add_limit(&solves("USD", *set_by, *amount), now())
            .unwrap();
    }
    let mut run = charging(&env, "solver");

    assert_eq!(run.allowed_by("paid_solves", "USD"), Ok(allowed_by));
}

#[test]
fn a_persons_limit_in_another_currency_allows_no_payment_in_this_one() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&solves("EUR", LimitSetter::Person, 10), now())
        .unwrap();
    let mut run = charging(&env, "solver");

    assert_eq!(run.allowed_by("paid_solves", "USD"), Ok(None));
}
