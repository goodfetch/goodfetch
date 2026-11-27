//! The model-based test of the Schedule store (ADR 0060; TESTING.md,
//! Model-based tests and crashes in one process): creates and deletes
//! through the store's writer actor on real SQLite over the crash VFS,
//! process crashes and power cuts between writes and during a create, each
//! followed by a new engine opening the store again. After every step the
//! store lists every Schedule the reference model acknowledged and nothing
//! else but creates a crash cut off, each create and delete answers as the
//! model says, and the store keeps the store ID its first create drew;
//! after every crash the file passes `integrity_check`. 16 cases in the
//! Quick suite and 256 in the Pre-merge suite (`proptest_config`). Each
//! case's folder lies in memory (`TestEnv::in_memory`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::user_store::UserStore;
use goodfetch_protocol::schedule::{ScheduleId, ScheduleTrigger};
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use tokio::runtime::Runtime;

use super::super::{STORE, create, delete, list};
use super::{drawn, schedule, store_id};

/// The Schedule IDs a case draws from, few so that creates collide.
const IDS: [&str; 5] = ["a", "b", "c", "d", "e"];

/// What the reference model knows: each acknowledged Schedule's interval
/// by ID, each create a crash cut off, and whether a create was
/// acknowledged, which drew the store ID.
#[derive(Clone, Debug, Default)]
struct Reference {
    acknowledged: BTreeMap<usize, u64>,
    uncertain: BTreeMap<usize, u64>,
    created: bool,
}

#[derive(Clone, Debug)]
enum Transition {
    /// A create of `IDS[id]` every `every` ms: recorded unless taken.
    Create { id: usize, every: u64 },
    /// A delete of `IDS[id]`.
    Delete { id: usize },
    /// A create of `IDS[id]` cut off by a crash, then a new engine.
    CrashDuringCreate {
        id: usize,
        every: u64,
        kind: CrashKind,
    },
    /// A crash between writes, then a new engine.
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
        let id = 0..IDS.len();
        prop_oneof![
            4 => (id.clone(), 1_u64..1_000).prop_map(|(id, every)| Transition::Create { id, every }),
            2 => id.clone().prop_map(|id| Transition::Delete { id }),
            1 => (id, 1_u64..1_000, crash_kind())
                .prop_map(|(id, every, kind)| Transition::CrashDuringCreate { id, every, kind }),
            1 => crash_kind().prop_map(Transition::Crash),
        ]
        .boxed()
    }

    /// A write to an ID a crash left uncertain would answer what the
    /// model cannot tell, so none is drawn.
    fn preconditions(state: &Self, transition: &Transition) -> bool {
        match transition {
            Transition::Create { id, .. }
            | Transition::Delete { id }
            | Transition::CrashDuringCreate { id, .. } => !state.uncertain.contains_key(id),
            Transition::Crash(_) => true,
        }
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        match transition {
            Transition::Create { id, every } => {
                state.acknowledged.entry(*id).or_insert(*every);
                state.created = true;
            }
            Transition::Delete { id } => {
                state.acknowledged.remove(id);
            }
            Transition::CrashDuringCreate { id, every, .. } => {
                if !state.acknowledged.contains_key(id) {
                    state.uncertain.insert(*id, *every);
                }
            }
            Transition::Crash(_) => {}
        }
        state
    }
}

/// The store under test, on the crash VFS, as one engine opened it.
struct Subject {
    runtime: Runtime,
    _env: TestEnv,
    vfs: CrashVfs,
    path: String,
    owner: Arc<TaskOwner>,
    store: Option<UserStore>,
    /// The draws given so far, each create's store ID draw its next one,
    /// so that a second draw would leave another ID.
    draws: u64,
    /// The model before the transition being applied.
    before: Reference,
    /// The store ID the first acknowledged create left.
    store_id: Option<String>,
}

impl Subject {
    fn open(&mut self) {
        self.owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        self.store = Some(UserStore::open(
            self.path.clone().into(),
            &STORE,
            Some(self.vfs.name().to_owned()),
            Arc::clone(&self.owner),
        ));
    }

    /// Stops the store's writer, as its engine's process ending does.
    fn stop(&mut self) {
        drop(self.store.take());
        self.runtime.block_on(self.owner.wait());
    }

    fn store(&self) -> UserStore {
        self.store.clone().expect("invariant: the store is open")
    }

    /// The next store ID draw.
    fn draw(&mut self) -> impl FnOnce() -> String + Send + 'static {
        self.draws += 1;
        drawn(self.draws)
    }

    /// Checks the crashed file as the next engine finds it, then opens the
    /// store again.
    fn reopen_after_crash(&mut self) {
        self.stop();
        drop(after_crash(&self.vfs, &self.path).expect("the store survives a crash undamaged"));
        self.open();
    }

    /// The store ID once a create was acknowledged, read the first time.
    fn note_store_id(&mut self) {
        if self.store_id.is_none() {
            let store = self.store();
            self.store_id = self.runtime.block_on(store_id(&store)).unwrap();
        }
    }
}

/// The listed intervals hold every acknowledged one and nothing else but
/// a create a crash cut off.
fn check(listed: &BTreeMap<usize, u64>, reference: &Reference) {
    for (id, every) in &reference.acknowledged {
        assert_eq!(
            listed.get(id),
            Some(every),
            "acknowledged `{}` is gone or changed: {listed:?}",
            IDS[*id]
        );
    }
    let allowed: BTreeSet<_> = reference
        .acknowledged
        .keys()
        .chain(reference.uncertain.keys())
        .collect();
    for (id, every) in listed {
        assert!(
            allowed.contains(id),
            "`{}` was never acknowledged",
            IDS[*id]
        );
        if let Some(cut_off) = reference.uncertain.get(id) {
            assert_eq!(every, cut_off, "`{}` holds what no create wrote", IDS[*id]);
        }
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
            env.unique_name("schedule-store-model"),
            folder.replace(' ', "-")
        );
        let vfs = CrashVfs::register(&name, env.seed()).unwrap();
        let path = env
            .folder()
            .join("schedule-store.sqlite")
            .to_string_lossy()
            .into_owned();
        let mut subject = Self {
            runtime: goodfetch_test_support::runtime(),
            _env: env,
            vfs,
            path,
            owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
            store: None,
            draws: 0,
            before: reference.clone(),
            store_id: None,
        };
        subject.open();
        // A first write makes the file, so a crash always finds one.
        let store = subject.store();
        let made = subject
            .runtime
            .block_on(delete(&store, &ScheduleId::parse("z").unwrap()))
            .unwrap();
        assert!(!made, "a new store held a Schedule");
        subject
    }

    fn apply(mut subject: Self, reference: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Create { id, every } => {
                let store = subject.store();
                let draw = subject.draw();
                let created = subject
                    .runtime
                    .block_on(create(&store, schedule(IDS[id], every), 0, draw))
                    .unwrap();
                assert_eq!(
                    created,
                    !subject.before.acknowledged.contains_key(&id),
                    "the create of `{}` answered otherwise than the model",
                    IDS[id]
                );
                subject.note_store_id();
            }
            Transition::Delete { id } => {
                let store = subject.store();
                let deleted = subject
                    .runtime
                    .block_on(delete(&store, &ScheduleId::parse(IDS[id]).unwrap()))
                    .unwrap();
                assert_eq!(
                    deleted,
                    subject.before.acknowledged.contains_key(&id),
                    "the delete of `{}` answered otherwise than the model",
                    IDS[id]
                );
            }
            Transition::CrashDuringCreate { id, every, kind } => {
                let store = subject.store();
                let draw = subject.draw();
                let vfs = &subject.vfs;
                // Acknowledged or not, the model counts it uncertain.
                let (_, ()) = subject.runtime.block_on(async {
                    tokio::join!(create(&store, schedule(IDS[id], every), 0, draw), async {
                        vfs.crash(kind);
                    })
                });
                drop(store);
                subject.reopen_after_crash();
            }
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.reopen_after_crash();
            }
        }
        subject.before = reference.clone();
        subject
    }

    fn check_invariants(subject: &Self, reference: &Reference) {
        let store = subject.store();
        let listed: BTreeMap<usize, u64> = subject
            .runtime
            .block_on(list(&store))
            .unwrap()
            .into_iter()
            .map(|info| {
                let id = IDS
                    .iter()
                    .position(|id| *id == info.schedule_id.as_str())
                    .expect("an ID of the case");
                let ScheduleTrigger::Every(every) = info.trigger else {
                    panic!("model test creates Every triggers only");
                };
                (id, every.every)
            })
            .collect();
        check(&listed, reference);
        let held = subject.runtime.block_on(store_id(&store)).unwrap();
        if reference.created {
            assert!(held.is_some(), "an acknowledged create left no store ID");
        }
        if let Some(first) = &subject.store_id {
            assert_eq!(Some(first), held.as_ref(), "the store ID changed");
        }
    }

    fn teardown(mut subject: Self, _: Reference) {
        subject.stop();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:10-7
    #[test]
    fn the_schedule_store_keeps_every_acknowledged_create_through_crashes_and_power_cuts(
        sequential 1..16 => Subject
    );
}
