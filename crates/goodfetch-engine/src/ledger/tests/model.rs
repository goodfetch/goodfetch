//! The model-based test of the Usage ledger (ADR 0060; TESTING.md,
//! Model-based tests and crashes in one process): two runs charge Usage
//! against one install Limit through their Slices, give them back when they
//! end, and die in process crashes and power cuts through the crash VFS,
//! after which the next engine to open the ledger reclaims their Slices from
//! what each journaled. The reference model is what each run acknowledged:
//! the Usage it journaled under its epoch, as its run state would hold it.
//! A charge may be journaled in a deferred write, as an attempt's claim is
//! at `full` (ADR 0138's note of #1926), within [`BEYOND`] since the run's
//! last synced journal: a power cut takes those journals, and the reclaim
//! then charges [`BEYOND`] more than the journal holds, at most each Slice.
//! After every step the Limit's charged and held Usage never exceed it, the
//! Usage charged is at least what the ended and reclaimed runs used, never
//! under-charged, and exactly that while no run deferred, never twice, and
//! after every crash the file passes `PRAGMA integrity_check` (in
//! `after_crash`). 16 cases in the Quick suite and 256 in the Pre-merge
//! suite (`proptest_config`).

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use jiff::Timestamp;
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

use super::{install, limit, placement};
use crate::ledger::{Charging, Holder, Ledger, Probed, Refusal, SliceId, SliceSizing};

/// The install Limit every case charges against.
const AMOUNT: u64 = 60;

/// How many runs charge it.
const RUNS: usize = 2;

/// What a run's deferred journals may charge since its last synced one, as
/// the run state's writer bounds them by `DEFERRED_CHARGES`.
const BEYOND: u64 = 8;

#[derive(Clone, Debug, Default)]
struct Reference;

#[derive(Clone, Debug)]
enum Transition {
    /// The run charges `amount` of `requests`, journaled in a deferred
    /// write when `deferred` and the bound allows it.
    Charge {
        run: usize,
        amount: u64,
        deferred: bool,
    },
    /// The run ends and gives back what is left of its Slices; it starts
    /// again under a later run epoch.
    End { run: usize },
    /// The engine dies; the next one reclaims its runs' Slices.
    Crash(CrashKind),
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self).boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            6 => (0..RUNS, 1_u64..6, any::<bool>()).prop_map(|(run, amount, deferred)| {
                Transition::Charge {
                    run,
                    amount,
                    deferred,
                }
            }),
            2 => (0..RUNS).prop_map(|run| Transition::End { run }),
            1 => prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)]
                .prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(state: Self, _: &Transition) -> Self {
        state
    }
}

/// One run: its charging under its current epoch, and what it journaled.
struct Run {
    epoch: u64,
    charging: Charging,
    journal: BTreeMap<SliceId, u64>,
    /// What its journal holds once a power cut took its deferred writes.
    synced: BTreeMap<SliceId, u64>,
    /// What its deferred writes since the last synced one charged.
    uncovered: u64,
    /// Whether it ever deferred a journal, as the run file declares.
    deferred: bool,
}

/// The ledger under test, on the crash VFS.
struct Subject {
    env: TestEnv,
    vfs: CrashVfs,
    ledger: Ledger,
    runs: Vec<Run>,
    /// The Usage the ended and reclaimed runs used.
    charged: u64,
    /// What the reclaims may have charged beyond it.
    over: u64,
}

fn now() -> Timestamp {
    "2026-03-10T15:00:00Z".parse().unwrap()
}

impl Subject {
    fn run(&self, index: usize, epoch: u64) -> Run {
        let run_id = format!("run-{index}");
        Run {
            epoch,
            charging: Charging::new(
                self.ledger.clone(),
                Holder {
                    run_id: run_id.clone(),
                    epoch,
                    folder: self.env.folder().join(run_id),
                },
                placement(Product::Collie),
                SliceSizing::default(),
            ),
            journal: BTreeMap::new(),
            synced: BTreeMap::new(),
            uncovered: 0,
            deferred: false,
        }
    }

    /// The Limit's charged and held Usage.
    fn counted(&self) -> (u64, u64) {
        let usage = self.ledger.usage(Product::Collie, now()).unwrap();
        (usage[0].charged, usage[0].held)
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(_: &Reference) -> Self {
        let env = TestEnv::new();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let name = format!(
            "{}-{}",
            env.unique_name("ledger-model"),
            folder.replace(' ', "-")
        );
        let vfs = CrashVfs::register(&name, env.seed()).unwrap();
        let ledger = Ledger::at(
            env.home().join("usage-ledger.sqlite"),
            Some(vfs.name().to_owned()),
        );
        ledger
            .add_limit(&limit(install(), "requests", AMOUNT), now())
            .unwrap();
        let mut subject = Self {
            env,
            vfs,
            ledger,
            runs: Vec::new(),
            charged: 0,
            over: 0,
        };
        subject.runs = (0..RUNS).map(|index| subject.run(index, 1)).collect();
        subject
    }

    fn apply(mut subject: Self, _: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Charge {
                run,
                amount,
                deferred,
            } => {
                let charged = subject.runs[run].charging.charge("requests", amount, now());
                let run = &mut subject.runs[run];
                match charged {
                    Ok(charged) => {
                        run.journal.extend(charged.used);
                        if deferred && run.uncovered + amount <= BEYOND {
                            run.uncovered += amount;
                            run.deferred = true;
                        } else {
                            run.synced.clone_from(&run.journal);
                            run.uncovered = 0;
                        }
                    }
                    Err(Refusal::Reached(_)) => {}
                    Err(other) => panic!("a charge failed: {other}"),
                }
            }
            Transition::End { run } => {
                let used: u64 = subject.runs[run].journal.values().sum();
                subject.runs[run].charging.give_back().unwrap();
                subject.charged += used;
                let epoch = subject.runs[run].epoch + 1;
                subject.runs[run] = subject.run(run, epoch);
            }
            Transition::Crash(kind) => {
                let (_, held) = subject.counted();
                subject.vfs.crash(kind);
                drop(after_crash(&subject.vfs, &subject.ledger.path().to_string_lossy()).unwrap());
                // What each run's file holds after the crash: a power cut
                // took its deferred journals since its last synced one.
                let journals: BTreeMap<(String, u64), (BTreeMap<SliceId, u64>, u64)> = subject
                    .runs
                    .iter()
                    .enumerate()
                    .map(|(index, run)| {
                        let kept = match kind {
                            CrashKind::PowerCut => run.synced.clone(),
                            CrashKind::Process => run.journal.clone(),
                        };
                        let beyond = if run.deferred { BEYOND } else { 0 };
                        ((format!("run-{index}"), run.epoch), (kept, beyond))
                    })
                    .collect();
                subject
                    .ledger
                    .reclaim(&|holder| {
                        let (used, beyond) = journals
                            .get(&(holder.run_id.clone(), holder.epoch))
                            .cloned()
                            .unwrap_or_default();
                        Probed::Gone { used, beyond }
                    })
                    .unwrap();
                let used: u64 = subject
                    .runs
                    .iter()
                    .flat_map(|run| run.journal.values())
                    .sum();
                subject.charged += used;
                if subject.runs.iter().any(|run| run.deferred) {
                    subject.over += held - used;
                }
                subject.runs = (0..RUNS)
                    .map(|index| {
                        let epoch = subject.runs[index].epoch + 1;
                        subject.run(index, epoch)
                    })
                    .collect();
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, _: &Reference) {
        let (charged, held) = subject.counted();
        assert!(
            charged + held <= AMOUNT,
            "charged {charged} and held {held} exceed the Limit of {AMOUNT}"
        );
        assert!(
            charged >= subject.charged,
            "the ledger charged {charged}, under the {} the ended and reclaimed runs used",
            subject.charged
        );
        assert!(
            charged <= subject.charged + subject.over,
            "the ledger charged {charged}, more than the {} the ended and reclaimed runs used and the {} their Slices held beyond it",
            subject.charged,
            subject.over
        );
        let used: u64 = subject
            .runs
            .iter()
            .flat_map(|run| run.journal.values())
            .sum();
        assert!(
            used <= held,
            "the live runs used {used}, more than the {held} they hold"
        );
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-69, story:3-72
    #[test]
    fn the_ledger_never_charges_past_its_limit_nor_twice_through_crashes_and_power_cuts(
        sequential 1..40 => Subject
    );
}
