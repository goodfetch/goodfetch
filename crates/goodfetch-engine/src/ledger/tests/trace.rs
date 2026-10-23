//! The Usage ledger's transitions as `tracing` events (ADR 0060): each one
//! the slices and reclaim model names (`test-support/models/slices/`), under
//! [`TARGET`], in the order the ledger took them, which TLC validates a
//! trace against.

use std::sync::mpsc;

use goodfetch_test_support::TestEnv;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{charging, install, ledger, limit, now};
use crate::ledger::{Probed, TARGET};

/// The `tracing` layer that keeps the `transition` of each event under
/// [`TARGET`].
struct Transitions(mpsc::Sender<String>);

struct Transition(Option<String>);

impl tracing::field::Visit for Transition {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "transition" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Transitions {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != TARGET {
            return;
        }
        let mut transition = Transition(None);
        event.record(&mut transition);
        if let Some(transition) = transition.0 {
            let _ = self.0.send(transition);
        }
    }
}

/// Covers: story:3-69, story:3-72
#[test]
fn the_ledger_traces_each_transition_the_slices_model_names() {
    let env = TestEnv::new();
    let (sender, received) = mpsc::channel();
    let subscriber = tracing_subscriber::registry().with(Transitions(sender));
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

    assert_eq!(
        received.try_iter().collect::<Vec<_>>(),
        [
            "set_limit",
            "take_slice",
            "charge",
            "take_slice",
            "charge",
            "give_back",
            "give_back",
            "take_slice",
            "charge",
            "mark_for_review",
        ]
    );
}
