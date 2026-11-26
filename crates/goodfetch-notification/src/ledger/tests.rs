//! The model-based test of the delivery ledger (ADR 0027; TESTING.md,
//! Model-based tests): claims of attempts, settlements and resumes of the
//! Run, each under a higher run epoch, on two events and two channels.
//! After every step the ledger holds what the reference model holds, each
//! claim answers as the model's does, an attempt is claimed at most once
//! through every resume, and a delivered, failed or uncertain event is
//! never claimed again: a resumed Run never repeats a delivery.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_test_support::TestEnv;
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use tokio::runtime::Runtime;

use super::{Claim, Entry, Ledger, Settled, TABLES};
use crate::message::EventId;

/// An event and a channel, each one of two.
type Key = (u8, u8);

/// How a claimed attempt ends in a step.
#[derive(Clone, Debug)]
enum How {
    Delivered,
    /// The next attempt, one past the claimed one, is sent at `at`.
    RetryAt {
        at: SystemTime,
    },
    Failed,
}

#[derive(Clone, Debug)]
enum Transition {
    Claim { key: Key, attempt: u32 },
    Settle { key: Key, how: How },
    Resume,
}

/// What the reference model knows: the run epoch of the engine that holds
/// the Run, each entry, and the answer the last claim gets.
#[derive(Clone, Debug)]
struct Reference {
    epoch: u64,
    entries: BTreeMap<Key, Entry>,
    claim: Option<Claim>,
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self {
            epoch: 1,
            entries: BTreeMap::new(),
            claim: None,
        })
        .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        let key = (0..2_u8, 0..2_u8);
        let how = prop_oneof![
            Just(How::Delivered),
            (0..100_000_u64).prop_map(|at| How::RetryAt {
                at: UNIX_EPOCH + Duration::from_millis(at),
            }),
            Just(How::Failed),
        ];
        prop_oneof![
            4 => (key.clone(), 0..4_u32).prop_map(|(key, attempt)| Transition::Claim { key, attempt }),
            4 => (key, how).prop_map(|(key, how)| Transition::Settle { key, how }),
            1 => Just(Transition::Resume),
        ]
        .boxed()
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        state.claim = None;
        match transition {
            Transition::Claim { key, attempt } => {
                let attempt = *attempt;
                let claimed = Entry::Sending {
                    attempt,
                    run_epoch: state.epoch,
                };
                let claim = match state.entries.get(key).cloned() {
                    None => Some(claimed),
                    Some(Entry::Waiting { attempt: next, .. }) if next == attempt => Some(claimed),
                    Some(Entry::Sending {
                        attempt: in_flight,
                        run_epoch,
                    }) if run_epoch < state.epoch => {
                        let uncertain = Entry::Uncertain { attempt: in_flight };
                        state.entries.insert(*key, uncertain.clone());
                        state.claim = Some(Claim::Held(uncertain));
                        return state;
                    }
                    Some(_) => None,
                };
                state.claim = Some(match claim {
                    Some(entry) => {
                        state.entries.insert(*key, entry);
                        Claim::Claimed
                    }
                    None => Claim::Held(state.entries[key].clone()),
                });
            }
            Transition::Settle { key, how } => {
                if let Some(Entry::Sending { attempt, run_epoch }) = state.entries.get(key).cloned()
                    && run_epoch == state.epoch
                {
                    let settled = match how {
                        How::Delivered => Entry::Delivered { attempt },
                        How::RetryAt { at } => Entry::Waiting {
                            attempt: attempt + 1,
                            at: *at,
                        },
                        How::Failed => Entry::Failed {
                            attempt,
                            failure: "gone".to_owned(),
                        },
                    };
                    state.entries.insert(*key, settled);
                }
            }
            Transition::Resume => state.epoch += 1,
        }
        state
    }
}

/// The ledger under test, in a run file reopened at each resume, and every
/// attempt it let this test claim.
struct Subject {
    env: TestEnv,
    runtime: Runtime,
    owner: TaskOwner,
    ledger: Option<Ledger>,
    claimed: BTreeSet<(Key, u32)>,
}

impl Subject {
    fn open(&mut self, epoch: u64) {
        let state = self
            .runtime
            .block_on(RunState::open(
                self.env.folder().join("run.sqlite"),
                &RunStateOptions::default(),
                vec![TABLES],
                epoch,
                &self.owner,
            ))
            .unwrap();
        self.ledger = Some(Ledger::new(state));
    }

    /// Stops the run file's writer, as the Run's engine stopping does.
    fn stop(&mut self) {
        drop(self.ledger.take());
        self.runtime.block_on(self.owner.wait());
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
    }

    fn ledger(&self) -> &Ledger {
        self.ledger.as_ref().unwrap()
    }
}

/// The event ID and channel name of `key`.
fn named(key: Key) -> (EventId, String) {
    (
        EventId::new(format!("event-{}", key.0)).unwrap(),
        format!("channel-{}", key.1),
    )
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(reference: &Reference) -> Self {
        let mut subject = Self {
            env: TestEnv::in_memory(),
            runtime: goodfetch_test_support::runtime(),
            owner: TaskOwner::root(BlockingPolicy::Inline),
            ledger: None,
            claimed: BTreeSet::new(),
        };
        subject.open(reference.epoch);
        subject
    }

    fn apply(mut subject: Self, reference: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Claim { key, attempt } => {
                let (event, channel) = named(key);
                let claim = subject
                    .runtime
                    .block_on(subject.ledger().claim(&event, &channel, attempt))
                    .unwrap();
                if claim == Claim::Claimed {
                    assert!(
                        subject.claimed.insert((key, attempt)),
                        "attempt {attempt} of {key:?} was claimed twice"
                    );
                }
                assert_eq!(Some(claim), reference.claim, "a claim of {key:?}");
            }
            Transition::Settle { key, how } => {
                let (event, channel) = named(key);
                let settled = match how {
                    How::Delivered => Settled::Delivered,
                    // The model's entry after the step names the next
                    // attempt; a settlement it ignored changes nothing.
                    How::RetryAt { at } => Settled::RetryAt {
                        attempt: match reference.entries.get(&key) {
                            Some(Entry::Waiting { attempt, .. }) => *attempt,
                            _ => 1,
                        },
                        at,
                    },
                    How::Failed => Settled::Failed("gone"),
                };
                subject
                    .runtime
                    .block_on(subject.ledger().settle(&event, &channel, settled))
                    .unwrap();
            }
            Transition::Resume => {
                subject.stop();
                subject.open(reference.epoch);
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, reference: &Reference) {
        for event in 0..2 {
            for channel in 0..2 {
                let key = (event, channel);
                let (event_id, name) = named(key);
                let held = subject
                    .runtime
                    .block_on(subject.ledger().entry(&event_id, &name))
                    .unwrap();
                assert_eq!(held.as_ref(), reference.entries.get(&key), "{key:?}");
            }
        }
    }

    fn teardown(mut subject: Self, _: Reference) {
        subject.stop();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:11-268
    #[test]
    fn the_ledger_claims_each_attempt_once_and_never_resends_a_settled_event_through_resumes(
        sequential 1..24 => Subject
    );
}
