//! The model-based test of `lease` (ADR 0060; TESTING.md, Model-based
//! tests and crashes in one process): a Run's work items in a run file on
//! real SQLite through the crash VFS, leased with the default 70 s lease on
//! the paused `Clock`, whose attempts commit, renew, release and end as the
//! generated steps say while the clock advances, and which may crash or
//! lose power. A reference model of the leases says whether each commit
//! counts: only one made while its lease holds, unreleased, and while the
//! run file still holds that lease of its work item; a late commit is
//! refused with `lease_lost`. After every step the run file holds each
//! counted commit's Records once and nothing of a refused one, and each
//! work item's status is the model's; after a crash, the file the next
//! process opens passes `PRAGMA integrity_check` (in `after_crash`) and
//! still holds every counted commit. 16 cases in the Quick suite and 256 in
//! the Pre-merge suite (`proptest_config`).

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::lease::LeaseCheck as _;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::foxhound::crawl::{PushedRecord, RequestSeed, Seed};
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::run::PointInTime;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::DEFAULT_LEASE;
use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::work_items::{self, Failure, Unsuccessful};

#[derive(Clone, Debug)]
enum Transition {
    /// The next pending work item is leased to a new attempt.
    Lease,
    /// The monotonic clock moves on this many seconds.
    Advance(u64),
    /// An attempt's lease is renewed for this many seconds from now.
    Renew(Index, u64),
    /// An attempt ends and gives its lease back.
    Release(Index),
    /// The driver settles an attempt whose lease ended: its work item
    /// returns to pending, when the run file still holds that lease.
    Settle(Index),
    /// An attempt commits this many Records.
    Commit(Index, u8),
    /// The process crashes or the power goes; the case ends with the file
    /// the next process opens.
    Crash(CrashKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pending,
    Leased,
    Succeeded,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
        }
    }
}

#[derive(Clone, Debug)]
struct Item {
    status: Status,
    epoch: u64,
}

#[derive(Clone, Debug)]
struct Attempt {
    item: usize,
    epoch: u64,
    /// When its lease ends, in seconds of the clock.
    expires_at: u64,
    released: bool,
}

impl Attempt {
    fn holds_at(&self, now: u64) -> bool {
        !self.released && now < self.expires_at
    }
}

/// What the leases must give: the clock, each work item and attempt, the
/// Records committed, and whether the last commit counted.
#[derive(Clone, Debug)]
struct Model {
    now: u64,
    items: Vec<Item>,
    attempts: Vec<Attempt>,
    committed: Vec<Value>,
    last_commit: Option<bool>,
    crashed: bool,
}

fn record(attempt: &Attempt, number: u8) -> Value {
    json!({ "item": attempt.item + 1, "epoch": attempt.epoch, "number": number })
}

impl ReferenceStateMachine for Model {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        (1_usize..4)
            .prop_map(|count| Self {
                now: 0,
                items: vec![
                    Item {
                        status: Status::Pending,
                        epoch: 0,
                    };
                    count
                ],
                attempts: Vec::new(),
                committed: Vec::new(),
                last_commit: None,
                crashed: false,
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            3 => Just(Transition::Lease),
            3 => (0_u64..80).prop_map(Transition::Advance),
            2 => (any::<Index>(), 1_u64..80).prop_map(|(attempt, seconds)| Transition::Renew(attempt, seconds)),
            1 => any::<Index>().prop_map(Transition::Release),
            2 => any::<Index>().prop_map(Transition::Settle),
            3 => (any::<Index>(), 0_u8..3).prop_map(|(attempt, count)| Transition::Commit(attempt, count)),
            1 => prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)].prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut model: Self, transition: &Transition) -> Self {
        model.last_commit = None;
        if model.crashed {
            return model;
        }
        let now = model.now;
        let chosen = |index: &Index, attempts: &[Attempt]| {
            (!attempts.is_empty()).then(|| index.index(attempts.len()))
        };
        match transition {
            Transition::Lease => {
                if let Some(item) = model
                    .items
                    .iter()
                    .position(|item| item.status == Status::Pending)
                {
                    let item_state = &mut model.items[item];
                    item_state.status = Status::Leased;
                    item_state.epoch += 1;
                    model.attempts.push(Attempt {
                        item,
                        epoch: item_state.epoch,
                        expires_at: now + DEFAULT_LEASE.as_secs(),
                        released: false,
                    });
                }
            }
            Transition::Advance(seconds) => model.now += seconds,
            Transition::Renew(index, seconds) => {
                if let Some(index) = chosen(index, &model.attempts) {
                    let attempt = &mut model.attempts[index];
                    if attempt.holds_at(now) {
                        attempt.expires_at = attempt.expires_at.max(now + seconds);
                    }
                }
            }
            Transition::Release(index) => {
                if let Some(index) = chosen(index, &model.attempts) {
                    let attempt = &mut model.attempts[index];
                    if attempt.holds_at(now) {
                        attempt.released = true;
                    }
                }
            }
            Transition::Settle(index) => {
                if let Some(index) = chosen(index, &model.attempts) {
                    let attempt = &model.attempts[index];
                    let item = &mut model.items[attempt.item];
                    if !attempt.holds_at(now)
                        && item.status == Status::Leased
                        && item.epoch == attempt.epoch
                    {
                        item.status = Status::Pending;
                    }
                }
            }
            Transition::Commit(index, count) => {
                if let Some(index) = chosen(index, &model.attempts) {
                    let attempt = model.attempts[index].clone();
                    let item = &mut model.items[attempt.item];
                    let counts = attempt.holds_at(now)
                        && item.status == Status::Leased
                        && item.epoch == attempt.epoch;
                    if counts {
                        item.status = Status::Succeeded;
                        model
                            .committed
                            .extend((0..*count).map(|number| record(&attempt, number)));
                    }
                    model.last_commit = Some(counts);
                }
            }
            Transition::Crash(_) => model.crashed = true,
        }
        model
    }
}

/// The leases under test: a run file on the crash VFS, its writer, and the
/// lease of each attempt, in grant order.
struct Subject {
    runtime: Runtime,
    _env: TestEnv,
    vfs: CrashVfs,
    path: String,
    owner: TaskOwner,
    state: Option<RunState>,
    leases: Leases,
    attempts: Vec<(String, WorkItemLease)>,
    finished: bool,
}

impl Subject {
    fn state(&self) -> &RunState {
        self.state
            .as_ref()
            .expect("invariant: the writer is open until the case ends")
    }

    fn attempt(&self, index: Index) -> Option<(String, WorkItemLease)> {
        (!self.attempts.is_empty()).then(|| self.attempts[index.index(self.attempts.len())].clone())
    }

    /// The writer stops and the test's tasks end.
    fn close(&mut self) {
        self.state = None;
        self.attempts.clear();
        self.runtime.block_on(self.owner.wait());
    }
}

/// Each Record's data, and each work item's status, as the file holds them.
fn stored(connection: &Connection) -> (Vec<Value>, Vec<String>) {
    let mut records = connection
        .prepare("SELECT data FROM records ORDER BY dataset, number")
        .unwrap();
    let records = records
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect();
    let mut statuses = connection
        .prepare("SELECT status FROM work_items ORDER BY position")
        .unwrap();
    let statuses = statuses
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (records, statuses)
}

fn sorted(mut values: Vec<Value>) -> Vec<Value> {
    values.sort_by_key(Value::to_string);
    values
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Model;

    fn init_test(model: &Model) -> Self {
        let env = TestEnv::new();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("lease-model-{folder}"), env.seed()).unwrap();
        let runtime = goodfetch_test_support::runtime();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let path = env.folder().join("run.sqlite");
        let seeds: Vec<Seed> = (1..=model.items.len())
            .map(|page| {
                Seed::Request(RequestSeed {
                    url: format!("https://shop.test/{page}"),
                    label: "page".to_owned(),
                    payload: None,
                    priority: 0,
                    forefront: false,
                    method: HttpMethod::get(),
                    body: None,
                    key: None,
                })
            })
            .collect();
        let (state, leases) = runtime.block_on(async {
            let state = RunState::open(
                path.clone(),
                &RunStateOptions {
                    vfs: Some(vfs.name().to_owned()),
                    ..RunStateOptions::default()
                },
                vec![
                    work_items::TABLES,
                    crate::labels::TABLES,
                    goodfetch_dataset::TABLES,
                    goodfetch_frontier::VISITED_TABLES,
                    goodfetch_frontier::SCOPE_TABLES,
                    goodfetch_frontier::BOUNDS_TABLES,
                    goodfetch_frontier::RULES_TABLES,
                    goodfetch_frontier::payload::TABLES,
                    goodfetch_frontier::sitemap::TABLES,
                    goodfetch_frontier::trap_links::TABLES,
                ],
                1,
                &owner,
            )
            .await
            .unwrap();
            state
                .write(move |connection| {
                    goodfetch_dataset::Datasets::declare(&[DatasetDeclaration {
                        name: "pages".to_owned(),
                        schema: None,
                        key: Vec::new(),
                    }])
                    .unwrap()
                    .record(connection)?;
                    work_items::admit_seeds(
                        connection,
                        &mut work_items::Positions::of(connection)?,
                        &goodfetch_frontier::Scope::default(),
                        &goodfetch_engine::egress::Policy::new(&["*"], &[] as &[&str]).unwrap(),
                        &seeds,
                    )
                    .map(drop)
                })
                .await
                .unwrap();
            (
                state,
                Leases::new(Clock::paused(env.start_time()), "model", 1),
            )
        });
        Self {
            runtime,
            _env: env,
            vfs,
            path: path.to_string_lossy().into_owned(),
            owner,
            state: Some(state),
            leases,
            attempts: Vec::new(),
            finished: false,
        }
    }

    fn apply(mut subject: Self, model: &Model, transition: Transition) -> Self {
        if subject.finished {
            return subject;
        }
        match transition {
            Transition::Lease => {
                let state = subject.state().clone();
                let leased = subject
                    .runtime
                    .block_on(state.write(|connection| {
                        let first: Vec<String> = connection
                            .prepare("SELECT id FROM work_items WHERE status = 'pending' ORDER BY position LIMIT 1")?
                            .query_map([], |row| row.get(0))?
                            .collect::<rusqlite::Result<_>>()?;
                        work_items::lease(connection, &first)
                    }))
                    .unwrap();
                for leased in leased {
                    let _clock = subject.runtime.enter();
                    let lease = subject.leases.grant(
                        &leased.crawl_request.id,
                        leased.lease_epoch,
                        DEFAULT_LEASE,
                    );
                    subject.attempts.push((leased.crawl_request.id, lease));
                }
            }
            Transition::Advance(seconds) => subject
                .runtime
                .block_on(tokio::time::advance(Duration::from_secs(seconds))),
            Transition::Renew(index, seconds) => {
                if let Some((_, lease)) = subject.attempt(index) {
                    let _clock = subject.runtime.enter();
                    lease.renew(Duration::from_secs(seconds));
                }
            }
            Transition::Release(index) => {
                if let Some((_, lease)) = subject.attempt(index) {
                    let _clock = subject.runtime.enter();
                    lease.release();
                }
            }
            Transition::Settle(index) => {
                if let Some((id, lease)) = subject.attempt(index)
                    && subject.runtime.block_on(async { lease.check() }).is_err()
                {
                    let epoch = model.attempts[index.index(model.attempts.len())].epoch;
                    let state = subject.state().clone();
                    subject
                        .runtime
                        .block_on(state.write(move |connection| {
                            work_items::end(
                                connection,
                                &id,
                                epoch,
                                Unsuccessful::Retried(Failure::new("handler_lost", "no answer")),
                                &ended_at(),
                            )
                        }))
                        .unwrap();
                }
            }
            Transition::Commit(index, count) => {
                if let Some((_, lease)) = subject.attempt(index) {
                    let attempt = &model.attempts[index.index(model.attempts.len())];
                    let records = (0..count)
                        .map(|number| PushedRecord {
                            dataset: "pages".to_owned(),
                            data: record(attempt, number),
                        })
                        .collect();
                    let (state, masks) = (subject.state().clone(), Masks::new());
                    let committed =
                        subject
                            .runtime
                            .block_on(lease.commit(&state, &masks, records, ended_at()));
                    let counted = match committed {
                        Ok(_) => true,
                        Err(CommitError::LeaseLost) => false,
                        Err(CommitError::RunState(error)) => panic!("the run file failed: {error}"),
                        Err(CommitError::Refused(error)) => {
                            panic!("a Record was refused: {error:?}")
                        }
                    };
                    assert_eq!(
                        Some(counted),
                        model.last_commit,
                        "the commit counted where the model says otherwise: {model:?}"
                    );
                }
            }
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.close();
                let connection = after_crash(&subject.vfs, &subject.path).unwrap();
                let (records, _) = stored(&connection);
                assert_eq!(sorted(records), sorted(model.committed.clone()));
                subject.finished = true;
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, model: &Model) {
        if subject.finished {
            return;
        }
        let connection = Connection::open_with_flags_and_vfs(
            &subject.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            subject.vfs.name(),
        )
        .unwrap();
        let (records, statuses) = stored(&connection);
        let expected: Vec<String> = model
            .items
            .iter()
            .map(|item| item.status.as_str().to_owned())
            .collect();
        assert_eq!(
            (sorted(records), statuses),
            (sorted(model.committed.clone()), expected)
        );
    }

    fn teardown(mut subject: Self, _: Model) {
        if !subject.finished {
            subject.close();
        }
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-40, story:3-41
    #[test]
    fn a_lease_counts_only_commits_made_while_it_holds_through_timeouts_renewals_and_crashes(
        sequential 1..30 => Subject
    );
}

/// When the model's attempts end: their outcomes are not its subject.
fn ended_at() -> PointInTime {
    PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned())
}
