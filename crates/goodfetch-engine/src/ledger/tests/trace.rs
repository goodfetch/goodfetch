//! The Usage ledger's transitions as `tracing` events (ADR 0060): each one
//! the slices and reclaim model names (`test-support/models/slices/`), under
//! [`TARGET`], in the order the ledger took them, which TLC validates a
//! trace against.

use goodfetch_test_support::{CapturedLog, TestEnv, model_trace};
use tracing_subscriber::layer::SubscriberExt as _;

use super::{charging, install, ledger, limit, now};
use crate::ledger::{Probed, TARGET};

/// The trace of two runs, as the model trace layer writes it: one gives
/// back its Slices, the other dies and its Slice is marked for review.
/// `just tlc slices` validates it against the slices model's trace spec.
const TWO_RUNS: &str = include_str!("trace/two-runs.ndjson");

/// Covers: story:3-69, story:3-72
#[test]
fn the_ledger_traces_each_transition_the_slices_model_names() {
    let env = TestEnv::new();
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber =
        tracing_subscriber::registry().with(model_trace(&[TARGET], move || writer.clone()));
    let _traced = tracing::subscriber::set_default(subscriber);
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 200), now())
        .unwrap();
    let mut ended = charging(&env, "ended");
    let mut dead = charging(&env, "dead");

    ended.charge("requests", 1, now()).unwrap();
    ended.charge("requests", 2, now()).unwrap();
    ended.give_back().unwrap();
    dead.charge("requests", 3, now()).unwrap();
    ledger.reclaim(&|_| Probed::Unreadable).unwrap();

    assert_eq!(String::from_utf8(captured.written()).unwrap(), TWO_RUNS);
}
