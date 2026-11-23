//! The model-based test of Checkpoints and resume (ADR 0060; TESTING.md,
//! Model-based tests and crashes in one process): commits to a run file on
//! the crash VFS, at `full` or below it at `batch`, attempts' writes, which
//! ride the writer's open batch below `full`, Checkpoints of it written to
//! real files beside, each sealing the open batch first as the Runs
//! registry does, process crashes and power cuts each followed by a resume
//! from the run file, and resumes from any Checkpoint taken so far, which
//! becomes the run file again. After every step the writer's view, and
//! after every crash the reopened file, holds every commit the reference
//! model acknowledged and nothing else but commits a crash cut off, each
//! once: a crash returns to the last commit at `full` and to the last
//! written batch below it, the open batch's answered writes gone. Every
//! Checkpoint holds every write answered before it, under the writer's
//! run epoch; a resume from a Checkpoint holds that Checkpoint's state, so
//! the work since is to be redone; and every resume takes a run epoch
//! higher than any the run file or the Checkpoint held. The Run's
//! Lifecycle and when a resume takes a run epoch come from the Run's
//! transition table (ADR 0128): a crash loses the Run's engine and a
//! resume takes its writer role again. `batch` runs here with no
//! threshold, so only a barrier, a seal or the writer's end writes its
//! batch. 16 cases in the Quick suite and 256 in the Pre-merge suite
//! (`proptest_config`).
//! Each case's folder lies in memory (`TestEnv::in_memory`): on a busy
//! disk its files' syncs took it near the Quick kill (fix-queue 123).

use std::path::PathBuf;

use goodfetch_protocol::run::Lifecycle;
use goodfetch_protocol::run::durability::BatchThreshold;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use tokio::runtime::Runtime;

use super::{ITEMS, insert, insert_run, items};
use crate::checkpoint::{install, open_checkpoint, write};
use crate::run_state::{self, Durability, RunState, RunStateOptions, Syncing};
use crate::runs::lifecycle::{self, Next, RunEvent};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// `batch`'s preset with no threshold: a barrier, a seal or the writer's
/// end writes its batch, never the clock.
const BATCH: Durability = Durability {
    pages: BatchThreshold::Never,
    interval: BatchThreshold::Never,
    bytes: BatchThreshold::Never,
    ..Durability::BATCH
};

/// What a Checkpoint held when it was taken: the values acknowledged and
/// those a crash cut off, and the writer's run epoch.
#[derive(Clone, Debug)]
struct Taken {
    acknowledged: Vec<i64>,
    uncertain: Vec<i64>,
    epoch: u64,
}

/// What the reference model knows: the Durability level, each value
/// written, each one answered in the writer's open batch, each one whose
/// commit a crash cut off, the writer's run epoch, the Run's Lifecycle and
/// each Checkpoint.
#[derive(Clone, Debug)]
struct Reference {
    durability: Durability,
    next: i64,
    acknowledged: Vec<i64>,
    /// The attempts' writes answered in the open batch, below `full`:
    /// the writer's view does not show them, a crash takes them, and a
    /// barrier, a seal or a clean stop writes them.
    open: Vec<i64>,
    uncertain: Vec<i64>,
    epoch: u64,
    lifecycle: Lifecycle,
    checkpoints: Vec<Taken>,
}

impl Reference {
    /// The open batch is written.
    fn write_open_batch(&mut self) {
        let open = std::mem::take(&mut self.open);
        self.acknowledged.extend(open);
    }

    /// `event`, as the Run's table says: whether it changed the Lifecycle.
    fn apply_event(&mut self, event: RunEvent) -> bool {
        match lifecycle::apply(self.lifecycle, event) {
            Ok(Next::To(next)) => {
                self.lifecycle = next;
                true
            }
            Ok(Next::Unchanged) => false,
            Err(refused) => panic!("the Run's table refused {refused}"),
        }
    }

    /// The Run's engine is lost, and a resume takes its writer role again
    /// at the run epoch past its own and `after`.
    fn lose_engine_and_resume(&mut self, after: u64) {
        self.apply_event(RunEvent::EngineLost);
        if self.apply_event(RunEvent::Resume) {
            self.epoch = self.epoch.max(after) + 1;
        }
    }
}

#[derive(Clone, Debug)]
enum Transition {
    /// `count` writes queued together, each a barrier that writes the open
    /// batch with it.
    Commit { count: usize },
    /// `count` attempts' writes, one after another, each ending its
    /// attempt: in the open batch below `full`, committed at `full`.
    Attempts { count: usize },
    /// One write queued, then a crash before the writer commits it, then a
    /// resume from the run file.
    CrashDuringCommit(CrashKind),
    /// A crash between commits, then a resume from the run file.
    Crash(CrashKind),
    /// A Checkpoint of the run file now, which seals the open batch first.
    Checkpoint,
    /// A resume from the Checkpoint taken `index` modulo how many were,
    /// which becomes the run file.
    ResumeFrom(usize),
}

fn crash_kind() -> impl Strategy<Value = CrashKind> {
    prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)]
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        prop_oneof![Just(Durability::FULL), Just(BATCH)]
            .prop_map(|durability| Self {
                durability,
                next: 0,
                acknowledged: Vec::new(),
                open: Vec::new(),
                uncertain: Vec::new(),
                epoch: 1,
                lifecycle: Lifecycle::Running,
                checkpoints: Vec::new(),
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            2 => (1_usize..4).prop_map(|count| Transition::Commit { count }),
            3 => (1_usize..4).prop_map(|count| Transition::Attempts { count }),
            1 => crash_kind().prop_map(Transition::CrashDuringCommit),
            1 => crash_kind().prop_map(Transition::Crash),
            2 => Just(Transition::Checkpoint),
            2 => (0_usize..8).prop_map(Transition::ResumeFrom),
        ]
        .boxed()
    }

    fn preconditions(state: &Self, transition: &Transition) -> bool {
        !matches!(transition, Transition::ResumeFrom(_)) || !state.checkpoints.is_empty()
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        match transition {
            Transition::Commit { count } => {
                state.write_open_batch();
                for index in 0..*count {
                    state
                        .acknowledged
                        .push(state.next + i64::try_from(index).unwrap());
                }
                state.next += i64::try_from(*count).unwrap();
            }
            Transition::Attempts { count } => {
                let batches = !state.durability.per_attempt;
                for index in 0..*count {
                    let value = state.next + i64::try_from(index).unwrap();
                    if batches {
                        state.open.push(value);
                    } else {
                        state.acknowledged.push(value);
                    }
                }
                state.next += i64::try_from(*count).unwrap();
            }
            // The barrier cut off was to write the open batch with it.
            Transition::CrashDuringCommit(_) => {
                let open = std::mem::take(&mut state.open);
                state.uncertain.extend(open);
                state.uncertain.push(state.next);
                state.next += 1;
                state.lose_engine_and_resume(0);
            }
            // The open batch is gone: the file is as of the last batch
            // written, or the last commit at `full`.
            Transition::Crash(_) => {
                state.open.clear();
                state.lose_engine_and_resume(0);
            }
            Transition::Checkpoint => {
                state.write_open_batch();
                state.apply_event(RunEvent::Checkpoint);
                state.checkpoints.push(Taken {
                    acknowledged: state.acknowledged.clone(),
                    uncertain: state.uncertain.clone(),
                    epoch: state.epoch,
                });
            }
            Transition::ResumeFrom(index) => {
                let taken = state.checkpoints[index % state.checkpoints.len()].clone();
                state.open.clear();
                state.acknowledged = taken.acknowledged;
                state.uncertain = taken.uncertain;
                state.lose_engine_and_resume(taken.epoch);
            }
        }
        state
    }
}

/// The run file under test, on the crash VFS, and its Checkpoints.
struct Subject {
    runtime: Runtime,
    env: TestEnv,
    vfs: CrashVfs,
    durability: Durability,
    path: String,
    owner: TaskOwner,
    state: Option<RunState>,
    checkpoints: Vec<PathBuf>,
}

impl Subject {
    fn options(&self) -> RunStateOptions {
        RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
            durability: self.durability,
            ..RunStateOptions::default()
        }
    }

    fn open(&mut self, epoch: u64) {
        let state = self
            .runtime
            .block_on(RunState::open(
                self.path.clone().into(),
                &self.options(),
                vec![ITEMS],
                epoch,
                &self.owner,
            ))
            .unwrap();
        self.state = Some(state);
    }

    /// Stops the writer, as its process ending does.
    fn stop(&mut self) {
        drop(self.state.take());
        self.runtime.block_on(self.owner.wait());
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
    }

    /// Takes the next run epoch past `after` and opens the writer under it,
    /// which must be `expected`.
    fn resume(&mut self, after: u64, expected: u64) {
        let epoch =
            run_state::next_epoch(self.path.as_ref(), &self.options(), &[ITEMS], after).unwrap();
        assert_eq!(epoch, expected, "the resume took another run epoch");
        self.open(epoch);
    }

    /// Checks the crashed file as the next process finds it, then resumes
    /// from it.
    fn resume_after_crash(&mut self, reference: &Reference) {
        self.stop();
        let connection = after_crash(&self.vfs, &self.path).unwrap();
        check(
            &items(&connection),
            &reference.acknowledged,
            &reference.uncertain,
        );
        drop::<Connection>(connection);
        self.resume(0, reference.epoch);
    }

    fn state(&self) -> &RunState {
        self.state.as_ref().unwrap()
    }
}

/// Every acknowledged value is there once, and nothing else but values a
/// crash cut off.
fn check(present: &[i64], acknowledged: &[i64], uncertain: &[i64]) {
    let mut seen = present.to_vec();
    seen.dedup();
    assert_eq!(
        seen.len(),
        present.len(),
        "a value is visible twice: {present:?}"
    );
    for value in acknowledged {
        assert!(
            present.contains(value),
            "acknowledged {value} is gone: {present:?}"
        );
    }
    for value in present {
        assert!(
            acknowledged.contains(value) || uncertain.contains(value),
            "{value} was never acknowledged: {present:?}"
        );
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(reference: &Reference) -> Self {
        let env = TestEnv::in_memory();
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
            env.unique_name("checkpoint-model"),
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
            env,
            vfs,
            durability: reference.durability,
            path,
            owner: TaskOwner::root(BlockingPolicy::Inline),
            state: None,
            checkpoints: Vec::new(),
        };
        subject.open(1);
        let state = subject.state().clone();
        subject
            .runtime
            .block_on(state.write(insert_run(1, None)))
            .unwrap();
        subject
    }

    fn apply(mut subject: Self, reference: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Commit { count } => {
                let first = reference.next - i64::try_from(count).unwrap();
                let state = subject.state().clone();
                subject.runtime.block_on(async {
                    for index in 0..count {
                        let value = first + i64::try_from(index).unwrap();
                        state.write(insert(value)).await.unwrap();
                    }
                });
            }
            Transition::Attempts { count } => {
                let first = reference.next - i64::try_from(count).unwrap();
                let state = subject.state().clone();
                let attempt = Syncing::Attempt { ended: 1, bytes: 0 };
                subject.runtime.block_on(async {
                    for index in 0..count {
                        let value = first + i64::try_from(index).unwrap();
                        state.write_with(attempt, insert(value)).await.unwrap();
                    }
                });
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
                subject.resume_after_crash(reference);
            }
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.resume_after_crash(reference);
            }
            Transition::Checkpoint => {
                let taken = &reference.checkpoints[reference.checkpoints.len() - 1];
                let destination = subject
                    .env
                    .folder()
                    .join(format!("checkpoint-{}.sqlite", subject.checkpoints.len()));
                let nonce = u64::try_from(subject.checkpoints.len()).unwrap();
                let state = subject.state().clone();
                subject.runtime.block_on(state.seal()).unwrap();
                let written = write(
                    subject.path.as_ref(),
                    Some(subject.vfs.name()),
                    &destination,
                    nonce,
                )
                .unwrap();
                assert_eq!(written.run_epoch, taken.epoch, "the Checkpoint's run epoch");
                let held = items(&open_checkpoint(&destination).unwrap());
                check(&held, &taken.acknowledged, &taken.uncertain);
                subject.checkpoints.push(destination);
            }
            Transition::ResumeFrom(index) => {
                let checkpoint = subject.checkpoints[index % subject.checkpoints.len()].clone();
                let before = subject.state().epoch();
                subject.stop();
                install(&checkpoint, subject.path.as_ref(), Some(subject.vfs.name())).unwrap();
                subject.resume(before, reference.epoch);
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, reference: &Reference) {
        let state = subject.state().clone();
        let present = subject
            .runtime
            .block_on(state.read(|connection| Ok(items(connection))))
            .unwrap();
        check(&present, &reference.acknowledged, &reference.uncertain);
        assert_eq!(state.epoch(), reference.epoch, "the writer's run epoch");
        let recorded = subject
            .runtime
            .block_on(state.read(|connection| {
                connection.query_row("SELECT lifecycle FROM run", [], |row| {
                    row.get::<_, String>(0)
                })
            }))
            .unwrap();
        assert_eq!(
            recorded,
            reference.lifecycle.as_str(),
            "the Run's Lifecycle"
        );
    }

    fn teardown(mut subject: Self, _: Reference) {
        subject.stop();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-30, story:3-34, setting:run_state.durability
    #[test]
    fn checkpoints_and_resumes_keep_every_acknowledged_commit_once_through_crashes_and_power_cuts(
        sequential 1..16 => Subject
    );
}
