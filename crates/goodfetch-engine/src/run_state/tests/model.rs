//! The model-based test of run state (ADR 0060; TESTING.md, Model-based
//! tests and crashes in one process): grouped commits, a failing write in a
//! group, commits cut off by a crash, and process crashes and power cuts
//! through the crash VFS, each followed by reopening the file. Each case
//! draws its layout, a run file of its own or one inside the install
//! database, which then opens through the crash VFS itself, and its
//! durability, `full` or `fast`. Its checkpointer copies the WAL past a
//! threshold of two pages, so it runs beside nearly every commit, and a
//! group may run a `PASSIVE` checkpoint of its own connection while the
//! writer holds the group's transaction: the checkpoint is never refused as
//! busy, and the writer commits every write of the group (ADR 0138). After every step the writer's view, and
//! after every crash the reopened file, holds nothing that was never
//! written, nothing twice, and every acknowledged commit, except those a
//! power cut may take in `fast` mode; and the run file, and the install
//! database in its layout, pass `PRAGMA integrity_check`. 16 cases in the
//! Quick suite and 256 in the Pre-merge suite (`proptest_config`).

use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use rusqlite::OpenFlags;
use tokio::runtime::Runtime;

use super::{ITEMS, insert, items};
use crate::run_state::install::{INSTALL_FILE, InstallDatabase};
use crate::run_state::{Durability, RunState, RunStateOptions};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// What the reference model knows: each value acknowledged, and each one
/// whose commit a crash cut off, which may or may not have landed.
#[derive(Clone, Debug)]
struct Reference {
    layout: Layout,
    durability: Durability,
    next: i64,
    acknowledged: Vec<i64>,
    uncertain: Vec<i64>,
}

/// Where the run file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// A file of its own, on the crash VFS.
    PerRun,
    /// Inside the install database, on the crash VFS.
    PerInstall,
}

#[derive(Clone, Debug)]
enum Transition {
    /// `count` writes queued together, the one at `failing` failing, the
    /// first running a checkpoint on a connection of its own when
    /// `checkpoint`.
    Commit {
        count: usize,
        failing: Option<usize>,
        checkpoint: bool,
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
        (
            prop_oneof![Just(Layout::PerRun), Just(Layout::PerInstall)],
            prop_oneof![Just(Durability::Full), Just(Durability::Fast)],
        )
            .prop_map(|(layout, durability)| Self {
                layout,
                durability,
                next: 0,
                acknowledged: Vec::new(),
                uncertain: Vec::new(),
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            4 => (1_usize..5, proptest::option::of(0_usize..5), any::<bool>()).prop_map(
                |(count, failing, checkpoint)| Transition::Commit {
                    count,
                    failing: failing.filter(|failing| *failing < count),
                    checkpoint,
                }
            ),
            1 => crash_kind().prop_map(Transition::CrashDuringCommit),
            1 => crash_kind().prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        match transition {
            Transition::Commit { count, failing, .. } => {
                for index in 0..*count {
                    let value = state.next + i64::try_from(index).unwrap();
                    if Some(index) != *failing {
                        state.acknowledged.push(value);
                    }
                }
                state.next += i64::try_from(*count).unwrap();
            }
            Transition::CrashDuringCommit(kind) => {
                state.uncertain.push(state.next);
                state.next += 1;
                state.crash(*kind);
            }
            Transition::Crash(kind) => state.crash(*kind),
        }
        state
    }
}

impl Reference {
    /// A power cut in `fast` mode may take acknowledged commits.
    fn crash(&mut self, kind: CrashKind) {
        if kind == CrashKind::PowerCut && self.durability == Durability::Fast {
            self.uncertain.append(&mut self.acknowledged);
        }
    }
}

/// The run file under test: its writer, on the crash VFS, directly or
/// through the install database.
struct Subject {
    runtime: Runtime,
    env: TestEnv,
    vfs: CrashVfs,
    layout: Layout,
    durability: Durability,
    path: String,
    install: Option<InstallDatabase>,
    owner: TaskOwner,
    state: Option<RunState>,
}

impl Subject {
    /// The VFS the run file opens through.
    fn run_file_vfs(&self) -> String {
        match &self.install {
            Some(install) => install.vfs().to_owned(),
            None => self.vfs.name().to_owned(),
        }
    }

    fn open(&mut self) {
        if self.layout == Layout::PerInstall {
            self.install = Some(
                InstallDatabase::open(
                    &self.install_path(),
                    &self.env.folder().join("runs"),
                    &RunStateOptions {
                        vfs: Some(self.vfs.name().to_owned()),
                        ..RunStateOptions::default()
                    },
                )
                .unwrap(),
            );
        }
        let options = RunStateOptions {
            vfs: Some(self.run_file_vfs()),
            durability: self.durability,
            wal_checkpoint_after: CHECKPOINT_AFTER,
            ..RunStateOptions::default()
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
        let connection = match self.install.take() {
            None => after_crash(&self.vfs, &self.path).unwrap(),
            Some(crashed) => {
                drop(crashed);
                let database = self.install_path().to_string_lossy().into_owned();
                drop::<Connection>(after_crash(&self.vfs, &database).unwrap());
                let install = InstallDatabase::open(
                    &self.install_path(),
                    &self.env.folder().join("runs"),
                    &RunStateOptions {
                        vfs: Some(self.vfs.name().to_owned()),
                        ..RunStateOptions::default()
                    },
                )
                .unwrap();
                let connection = checked(&self.path, install.vfs());
                drop(install);
                connection
            }
        };
        // A power cut in `fast` mode may take even the commit that made the
        // table.
        let present = items(&connection).unwrap_or_else(|error| {
            assert!(error.to_string().contains("no such table"), "{error}");
            Vec::new()
        });
        check(&present, reference);
        drop::<Connection>(connection);
        self.open();
    }

    fn install_path(&self) -> std::path::PathBuf {
        self.env.folder().join(INSTALL_FILE)
    }

    fn state(&self) -> &RunState {
        self.state.as_ref().unwrap()
    }
}

/// The checkpointer's threshold in the model: two pages of WAL.
const CHECKPOINT_AFTER: u64 = 2 * 4096;

/// Runs a `PASSIVE` checkpoint of the run file at `path` on a connection of
/// its own through `vfs`, as the checkpointer does, and answers whether it
/// was refused as busy (1) or not (0).
fn passive_checkpoint(path: &str, vfs: &str) -> i64 {
    let connection = Connection::open_with_flags_and_vfs(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs,
    )
    .unwrap();
    connection
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| row.get(0))
        .unwrap()
}

/// Opens the run file at `path` through `vfs` as the next process would,
/// and checks it with `PRAGMA integrity_check`.
fn checked(path: &str, vfs: &str) -> Connection {
    let connection = Connection::open_with_flags_and_vfs(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs,
    )
    .unwrap();
    let integrity: String = connection
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    connection
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

    fn init_test(reference: &Reference) -> Self {
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
            .join("runs")
            .join("r")
            .join("run.sqlite")
            .to_string_lossy()
            .into_owned();
        let mut subject = Self {
            runtime: goodfetch_test_support::runtime(),
            env,
            vfs,
            layout: reference.layout,
            durability: reference.durability,
            path,
            install: None,
            owner: TaskOwner::root(BlockingPolicy::Inline),
            state: None,
        };
        subject.open();
        subject
    }

    fn apply(mut subject: Self, reference: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Commit {
                count,
                failing,
                checkpoint,
            } => {
                let first = reference.next - i64::try_from(count).unwrap();
                let state = subject.state().clone();
                let (path, vfs) = (subject.path.clone(), subject.run_file_vfs());
                let answers = subject.runtime.block_on(async {
                    let writes = (0..count).map(|index| {
                        let state = state.clone();
                        let value = first + i64::try_from(index).unwrap();
                        let fails = Some(index) == failing;
                        let checkpoints = checkpoint && index == 0;
                        let (path, vfs) = (path.clone(), vfs.clone());
                        async move {
                            state
                                .write(move |connection| {
                                    insert(value)(connection)?;
                                    let busy = if checkpoints {
                                        passive_checkpoint(&path, &vfs)
                                    } else {
                                        0
                                    };
                                    if fails {
                                        return Err(rusqlite::Error::InvalidQuery);
                                    }
                                    Ok(busy)
                                })
                                .await
                        }
                    });
                    join_all(writes).await
                });
                for (index, answer) in answers.iter().enumerate() {
                    assert_eq!(answer.is_ok(), Some(index) != failing, "{answers:?}");
                }
                assert!(
                    answers
                        .iter()
                        .all(|answer| answer.as_ref().map_or(true, |busy| *busy == 0)),
                    "a checkpoint beside the writer's transaction was refused as busy: {answers:?}"
                );
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
        drop(subject.install.take());
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

    /// Covers: story:3-25, story:3-26, story:3-27, story:3-29
    #[test]
    fn run_state_keeps_every_acknowledged_commit_once_through_crashes_and_power_cuts(
        sequential 1..24 => Subject
    );
}
