//! The model-based test of run state (ADR 0060; TESTING.md, Model-based
//! tests and crashes in one process): grouped commits, a failing write in a
//! group, commits cut off by a crash, and process crashes and power cuts
//! through the crash VFS, each followed by reopening the file. After every
//! step the writer's view, and after every crash the reopened file, holds
//! every acknowledged commit, nothing that was never written, nothing twice,
//! and passes `PRAGMA integrity_check` (in `after_crash`). 16 cases in the
//! Quick suite and 256 in the Pre-merge suite (`proptest_config`).

use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use tokio::runtime::Runtime;

use super::{ITEMS, insert, items};
use crate::run_state::{RunState, RunStateOptions};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// What the reference model knows: each value acknowledged, and each one
/// whose commit a crash cut off, which may or may not have landed.
#[derive(Clone, Debug, Default)]
struct Reference {
    next: i64,
    acknowledged: Vec<i64>,
    uncertain: Vec<i64>,
}

#[derive(Clone, Debug)]
enum Transition {
    /// `count` writes queued together, the one at `failing` failing.
    Commit {
        count: usize,
        failing: Option<usize>,
    },
    /// One write queued, then a crash before the writer commits it.
    CrashDuringCommit(CrashKind),
    /// A crash between commits, then the file opens again.
    Crash(CrashKind),
}

fn crash_kind() -> impl Strategy<Value = CrashKind> {
    prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)]
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self::default()).boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            4 => (1_usize..5, proptest::option::of(0_usize..5)).prop_map(|(count, failing)| {
                Transition::Commit {
                    count,
                    failing: failing.filter(|failing| *failing < count),
                }
            }),
            1 => crash_kind().prop_map(Transition::CrashDuringCommit),
            1 => crash_kind().prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        match transition {
            Transition::Commit { count, failing } => {
                for index in 0..*count {
                    let value = state.next + i64::try_from(index).unwrap();
                    if Some(index) != *failing {
                        state.acknowledged.push(value);
                    }
                }
                state.next += i64::try_from(*count).unwrap();
            }
            Transition::CrashDuringCommit(_) => {
                state.uncertain.push(state.next);
                state.next += 1;
            }
            Transition::Crash(_) => {}
        }
        state
    }
}

/// The run file under test: its writer, on the crash VFS.
struct Subject {
    runtime: Runtime,
    _env: TestEnv,
    vfs: CrashVfs,
    path: String,
    owner: TaskOwner,
    state: Option<RunState>,
}

impl Subject {
    fn open(&mut self) {
        let options = RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
        };
        let state = self
            .runtime
            .block_on(RunState::open(
                self.path.clone().into(),
                &options,
                vec![ITEMS],
                1,
                &self.owner,
            ))
            .unwrap();
        self.state = Some(state);
    }

    /// Stops the crashed writer, checks the file as the next process finds
    /// it, then opens it again.
    fn reopen(&mut self, reference: &Reference) {
        drop(self.state.take());
        self.runtime.block_on(self.owner.wait());
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
        let connection = after_crash(&self.vfs, &self.path).unwrap();
        check(&items(&connection).unwrap(), reference);
        drop::<Connection>(connection);
        self.open();
    }

    fn state(&self) -> &RunState {
        self.state.as_ref().unwrap()
    }
}

/// Every acknowledged value is there once, and nothing else but values a
/// crash cut off.
fn check(present: &[i64], reference: &Reference) {
    let mut seen = present.to_vec();
    seen.dedup();
    assert_eq!(
        seen.len(),
        present.len(),
        "a value is visible twice: {present:?}"
    );
    for value in &reference.acknowledged {
        assert!(
            present.contains(value),
            "acknowledged {value} is gone: {present:?}"
        );
    }
    for value in present {
        assert!(
            reference.acknowledged.contains(value) || reference.uncertain.contains(value),
            "{value} was never acknowledged: {present:?}"
        );
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(_: &Reference) -> Self {
        let env = TestEnv::new();
        // Each case's folder has a name of its own; the VFS takes it, since
        // every case of the test registers one in the same process.
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let name = format!(
            "{}-{}",
            env.unique_name("run-state-model"),
            folder.replace(' ', "-")
        );
        let vfs = CrashVfs::register(&name, env.seed()).unwrap();
        let path = env
            .folder()
            .join("run.sqlite")
            .to_string_lossy()
            .into_owned();
        let mut subject = Self {
            runtime: goodfetch_test_support::runtime(),
            _env: env,
            vfs,
            path,
            owner: TaskOwner::root(BlockingPolicy::Inline),
            state: None,
        };
        subject.open();
        subject
    }

    fn apply(mut subject: Self, reference: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Commit { count, failing } => {
                let first = reference.next - i64::try_from(count).unwrap();
                let state = subject.state().clone();
                let answers = subject.runtime.block_on(async {
                    let writes = (0..count).map(|index| {
                        let state = state.clone();
                        let value = first + i64::try_from(index).unwrap();
                        let fails = Some(index) == failing;
                        async move {
                            state
                                .write(move |connection| {
                                    insert(value)(connection)?;
                                    if fails {
                                        return Err(rusqlite::Error::InvalidQuery);
                                    }
                                    Ok(())
                                })
                                .await
                        }
                    });
                    join_all(writes).await
                });
                for (index, answer) in answers.iter().enumerate() {
                    assert_eq!(answer.is_ok(), Some(index) != failing, "{answers:?}");
                }
            }
            Transition::CrashDuringCommit(kind) => {
                let value = reference.next - 1;
                let state = subject.state().clone();
                let vfs = &subject.vfs;
                let (answer, ()) = subject.runtime.block_on(async {
                    tokio::join!(state.write(insert(value)), async {
                        vfs.crash(kind);
                    })
                });
                assert!(
                    answer.is_err(),
                    "a commit cut off by a crash was acknowledged"
                );
                drop(state);
                subject.reopen(reference);
            }
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.reopen(reference);
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, reference: &Reference) {
        let state = subject.state().clone();
        let present = subject.runtime.block_on(state.read(items)).unwrap();
        check(&present, reference);
    }

    fn teardown(mut subject: Self, _: Reference) {
        drop(subject.state.take());
        subject.runtime.block_on(subject.owner.wait());
    }
}

/// Awaits every future of `futures` on this task, in order of their
/// answers.
async fn join_all<F: Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let futures: Vec<_> = futures.map(Box::pin).collect();
    let mut answers: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
    let mut futures: Vec<Option<_>> = futures.into_iter().map(Some).collect();
    std::future::poll_fn(|context| {
        let mut pending = false;
        for (slot, answer) in futures.iter_mut().zip(answers.iter_mut()) {
            if let Some(future) = slot {
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(output) => {
                        *answer = Some(output);
                        *slot = None;
                    }
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    answers.into_iter().map(Option::unwrap).collect()
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-25, story:3-26
    #[test]
    fn run_state_keeps_every_acknowledged_commit_once_through_crashes_and_power_cuts(
        sequential 1..24 => Subject
    );
}
