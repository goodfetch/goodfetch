//! The model-based test of delivery (ADR 0060; TESTING.md, Model-based
//! tests and crashes in one process): a crawl on real SQLite through the
//! crash VFS, on the test `Clock`, whose handler answers each attempt as
//! the generated script says (Records, a retryable failure, its own
//! failure, silence until the lease ends, or Records sent only once the
//! engine cancelled the attempt at its lease's end: a late commit, which
//! commits nothing, or Records whose write the run file refuses, an
//! outcome not written, which ends as a failed attempt), and which may crash or lose power between attempts, or
//! abort the Run. Some work items' attempts panic inside the engine, as a
//! bug would, before their handler request goes (issue 2285): each such
//! item is tried again up to `crawl.retry.max`, with no handler request,
//! and ends `failed` with `engine_panicked`, while the others go on. A reference model of the Run's policy says which work item
//! and attempt each handler request must be, and which Records each commit
//! holds; the Run's transition table (ADR 0128) is its reference for the
//! Run's Lifecycle: the driver's finish once no work is left, an abort, the
//! loss of the engine in a crash, and a finish or an abort of the Run
//! afterwards, which another engine's abort takes over after a crash. After every step the run file holds every Record
//! acknowledged, none twice and none of an attempt that did not commit,
//! and each of its two Datasets numbers its Records from 1, in commit
//! order, with no gaps (ADRs 0008 and 0103);
//! after a crash, the file the next process opens passes `PRAGMA
//! integrity_check` (in `after_crash`) and still does. The handler takes
//! one work item at a time, so a handler request means the previous
//! attempt's outcome was answered. Each case runs at a level it draws
//! (`run_state.durability`): at `full` that outcome is in the file; at
//! `batch` it is in the open batch, which the interval, the abort's write
//! or the Run's end writes, so the file holds for sure only the Records
//! answered before the last such barrier the model knows of, the abort,
//! and the others it shows are each Dataset's next ones in commit order.
//! 16 cases in the Quick suite and 256 in the Pre-merge suite
//! (`proptest_config`).
//! Each case's folder lies in memory (`TestEnv::in_memory`): on a busy
//! disk its files' syncs passed the Quick kill (fix-queue 123).

use std::collections::BTreeMap;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::lifecycle::{self, Next, RunEvent};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Network};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_protocol::run::durability::MEMORY_BUDGET;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::runtime::Runtime;

use super::{
    PANICS, POLICY, as_sent, configure, default_dependencies, dependencies, engine, work_items,
};
use crate::policy::CrawlPolicy;

/// The model's crawls take one work item at a time.
const ONE_AT_A_TIME: CrawlPolicy = CrawlPolicy {
    handler_concurrency: 1,
    ..POLICY
};

/// What the handler does with the next attempt.
#[derive(Clone, Copy, Debug)]
enum Behaviour {
    /// Answers this many Records.
    Records(u8),
    /// Fails with a retryable Error code.
    FailsRetryably,
    /// Fails with its own `handler_failed`.
    FailsItself,
    /// Never answers, so the lease ends.
    Silent,
    /// Answers this many Records only once the engine cancelled the
    /// attempt at its lease's end: a late commit, which commits nothing.
    Late(u8),
    /// Answers this many Records, one at least, whose write the run file
    /// refuses: the attempt's outcome is not written, so it commits
    /// nothing and ends as a failed attempt, retried while its work item
    /// has retries left (issue 2320).
    Unwritten(u8),
}

#[derive(Clone, Debug)]
enum Transition {
    /// The next handler request is answered so.
    Deliver(Behaviour),
    /// The Client aborts the Run.
    Abort,
    /// The process crashes or the power goes, between attempts; the case
    /// ends with the file the next process opens.
    Crash(CrashKind),
}

/// One work item as the policy moves it; one whose attempts panic is
/// never handed to the handler.
#[derive(Clone, Debug, PartialEq)]
struct Item {
    attempts: u32,
    done: bool,
    panics: bool,
}

/// What the Run must hold: each work item, the Records committed for sure,
/// and those of the last answer, whose commit a crash may have cut off.
#[derive(Clone, Debug)]
struct Model {
    items: Vec<Item>,
    delivered: usize,
    committed: Vec<Value>,
    last_answer: Vec<Value>,
    crashed: bool,
    /// The Run's Lifecycle, as the Run's table changes it.
    lifecycle: Lifecycle,
    /// Whether the Run batches attempts (`batch`) or writes each attempt's
    /// outcome as it is answered (`full`).
    batching: bool,
    /// The bytes of what it wrote that the Run keeps in memory
    /// (`run_state.memory.budget`): none, a few visits' worth, or the
    /// default, which an outcome never shows (#2169).
    budget: u64,
    /// How many of `committed` a barrier wrote, at `batch`.
    sealed: usize,
}

impl Model {
    /// The Records the file holds for sure: every committed one at `full`,
    /// those a barrier wrote at `batch`.
    fn sure(&self) -> &[Value] {
        if self.batching {
            &self.committed[..self.sealed]
        } else {
            &self.committed
        }
    }

    /// The work item the next handler request leases, and its attempt;
    /// none once the Run ended. A work item whose attempts panic ends
    /// before any handler request, so none leases it.
    fn next(&self) -> Option<(usize, u32)> {
        if self.lifecycle != Lifecycle::Running {
            return None;
        }
        self.items
            .iter()
            .position(|item| !item.done && !item.panics)
            .map(|index| (index, self.items[index].attempts + 1))
    }

    /// The Lifecycle `events` leave the Run in, as the Run's table says.
    fn after(&self, events: &[RunEvent]) -> Lifecycle {
        events.iter().fold(self.lifecycle, |state, event| {
            match lifecycle::apply(state, *event) {
                Ok(Next::To(next)) => next,
                Ok(Next::Unchanged) => state,
                Err(refused) => panic!("the Run's table refused {refused}"),
            }
        })
    }

    /// The Lifecycles a crash may leave the Run in, as another engine's
    /// abort then reports it: its engine lost while it ran, or, once no
    /// work was left, after its driver's finish had landed.
    fn after_a_crash_and_an_abort(&self) -> Vec<Lifecycle> {
        let mut possible = vec![self.after(&[RunEvent::EngineLost, RunEvent::Abort])];
        if self.next().is_none() {
            possible.push(self.after(&[RunEvent::Finish, RunEvent::Abort]));
        }
        possible
    }
}

fn record(item: usize, attempt: u32, number: u8) -> Value {
    json!({ "item": item + 1, "attempt": attempt, "number": number })
}

/// The Dataset a handler pushes `record` into: `pages` for its even
/// numbers, `links` for its odd ones, so an attempt may push into both.
fn dataset_of(record: &Value) -> &'static str {
    if record["number"].as_u64().unwrap().is_multiple_of(2) {
        "pages"
    } else {
        "links"
    }
}

/// The handler's answer of `records`, each into its Dataset.
fn pushed(records: impl Iterator<Item = Value>) -> Vec<Value> {
    records
        .map(|data| json!({ "dataset": dataset_of(&data), "data": data }))
        .collect()
}

impl ReferenceStateMachine for Model {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        (
            prop::collection::vec(prop::bool::weighted(0.25), 1..4),
            any::<bool>(),
            prop::sample::select(vec![0, 512, MEMORY_BUDGET]),
        )
            .prop_map(|(panics, batching, budget)| Self {
                items: panics
                    .into_iter()
                    .map(|panics| Item {
                        attempts: 0,
                        done: false,
                        panics,
                    })
                    .collect(),
                delivered: 0,
                committed: Vec::new(),
                last_answer: Vec::new(),
                crashed: false,
                lifecycle: Lifecycle::Running,
                batching,
                budget,
                sealed: 0,
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            3 => (0_u8..3).prop_map(|count| Transition::Deliver(Behaviour::Records(count))),
            1 => Just(Transition::Deliver(Behaviour::FailsRetryably)),
            1 => Just(Transition::Deliver(Behaviour::FailsItself)),
            1 => Just(Transition::Deliver(Behaviour::Silent)),
            1 => (1_u8..3).prop_map(|count| Transition::Deliver(Behaviour::Late(count))),
            1 => (1_u8..3).prop_map(|count| Transition::Deliver(Behaviour::Unwritten(count))),
            1 => prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)].prop_map(Transition::Crash),
            1 => Just(Transition::Abort),
        ]
        .boxed()
    }

    /// An abort races nothing: it is sent only while the Run runs, before
    /// its driver could finish it.
    fn preconditions(model: &Self, transition: &Transition) -> bool {
        !matches!(transition, Transition::Abort) || model.next().is_some()
    }

    fn apply(mut model: Self, transition: &Transition) -> Self {
        if model.crashed {
            return model;
        }
        let behaviour = match transition {
            Transition::Deliver(behaviour) => behaviour,
            Transition::Abort => {
                model.lifecycle = model.after(&[RunEvent::Abort]);
                // The abort's write is a barrier: it writes every outcome
                // answered before it.
                model.sealed = model.committed.len();
                return model;
            }
            Transition::Crash(_) => {
                model.crashed = true;
                return model;
            }
        };
        let Some((index, attempt)) = model.next() else {
            return model;
        };
        model.delivered += 1;
        model.committed.append(&mut model.last_answer);
        let item = &mut model.items[index];
        item.attempts = attempt;
        match behaviour {
            Behaviour::Records(count) => {
                item.done = true;
                model.last_answer = (0..*count)
                    .map(|number| record(index, attempt, number))
                    .collect();
            }
            Behaviour::FailsRetryably
            | Behaviour::Silent
            | Behaviour::Late(_)
            | Behaviour::Unwritten(_) => {
                item.done = !ONE_AT_A_TIME.may_retry(attempt);
            }
            Behaviour::FailsItself => item.done = true,
        }
        model
    }
}

/// The crawl under test: an engine on the crash VFS, and its Client.
struct Subject {
    runtime: Runtime,
    env: TestEnv,
    vfs: CrashVfs,
    owner: TaskOwner,
    lines: Lines<BufReader<goodfetch_engine::transport::ConnectionReader>>,
    writer: goodfetch_engine::transport::ConnectionWriter,
    delivered: usize,
    finished: bool,
    /// Whether the Client aborted the Run.
    aborted: bool,
    /// The requests left unanswered, with the Records each answers once
    /// the engine cancels it.
    silent: Vec<(Value, Vec<Value>)>,
}

impl Subject {
    fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.runtime.block_on(self.writer.write_all(&line)).unwrap();
    }

    fn next(&mut self) -> Value {
        let line = self
            .runtime
            .block_on(self.lines.next_line())
            .unwrap()
            .expect("the engine closed early");
        serde_json::from_str(&line).unwrap()
    }

    fn path(&self) -> String {
        self.env
            .home()
            .join("1")
            .join("runs")
            .join("model")
            .join("run.sqlite")
            .to_string_lossy()
            .into_owned()
    }

    /// Says `hello` to the engine.
    fn hello(&mut self) {
        self.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": { "protocol_version": "0.1", "surface": "rust", "needs": "foxhound", "product_version": "1.4.0" },
        }));
        assert_eq!(self.next()["result"]["product"], "foxhound");
    }

    /// Calls `method` on the Run under the id `id` and answers its reply;
    /// a handler request on the way is left unanswered, as a request whose
    /// cancel the reply follows.
    fn call(&mut self, id: &str, method: &str) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": { "run_id": "model" } }));
        loop {
            let message = next_from_engine(self);
            if message["id"] == id {
                return message;
            }
            if message["method"] == "crawl_request.handle" {
                self.silent.push((message["id"].clone(), Vec::new()));
            }
        }
    }

    /// Starts another engine in the same home, on the crash VFS, and says
    /// `hello` to it.
    fn restart(&mut self) {
        let options = RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
            ..RunStateOptions::default()
        };
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
        let (env, owner) = (&self.env, &self.owner);
        let (reader, writer) = self
            .runtime
            .block_on(async { attach(owner, default_dependencies(env, options)).await });
        self.lines = BufReader::new(reader).lines();
        self.writer = writer;
        self.silent.clear();
        self.hello();
    }

    /// Has the run file refuse the Records of the `attempt`th attempt of
    /// the work item at `index`, through a trigger planted beside the
    /// writer: a write the run file refuses, with no storage failure. At
    /// `batch` the writer holds the open batch's transaction until the
    /// interval writes it, which only the engine's runtime, on the paused
    /// clock, brings: each try that finds the file busy lets it run.
    fn refuse_records(&self, index: usize, attempt: u32) {
        let connection = Connection::open_with_flags_and_vfs(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            self.vfs.name(),
        )
        .unwrap();
        connection.busy_timeout(std::time::Duration::ZERO).unwrap();
        let item = index + 1;
        let trigger = format!(
            "CREATE TRIGGER IF NOT EXISTS refuse_{item}_{attempt} BEFORE INSERT ON records
             WHEN NEW.work_item = '{item}'
                 AND (SELECT attempts FROM work_items WHERE id = NEW.work_item) = {attempt}
             BEGIN SELECT RAISE(ABORT, 'refused by the model'); END"
        );
        for _ in 0..600 {
            match connection.execute_batch(&trigger) {
                Ok(()) => return,
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::DatabaseBusy =>
                {
                    self.runtime.block_on(async {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    });
                }
                Err(error) => panic!("the trigger was not planted: {error}"),
            }
        }
        panic!("the run file stayed busy for a minute of the paused clock");
    }

    /// The Records the run file holds now, read beside the writer.
    fn records(&self) -> Vec<Stored> {
        let connection = Connection::open_with_flags_and_vfs(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            self.vfs.name(),
        )
        .unwrap();
        stored_records(&connection)
    }

    /// Each work item the run file holds now, read beside the writer.
    fn work_items(&self) -> Vec<(String, String, i64, Option<String>)> {
        let connection = Connection::open_with_flags_and_vfs(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            self.vfs.name(),
        )
        .unwrap();
        work_items(&connection)
    }

    /// Closes the Connection and waits for the engine to stop.
    fn stop(&mut self) {
        self.runtime.block_on(self.writer.shutdown()).unwrap();
        while self
            .runtime
            .block_on(self.lines.next_line())
            .unwrap()
            .is_some()
        {}
        self.runtime.block_on(self.owner.wait());
    }
}

/// One committed Record: its Dataset, its number and its data.
type Stored = (String, i64, Value);

fn stored_records(connection: &Connection) -> Vec<Stored> {
    let mut statement = connection
        .prepare("SELECT dataset, number, data FROM records ORDER BY dataset, number")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .map(|row| {
            let (dataset, number, data) = row.unwrap();
            (dataset, number, serde_json::from_str(&data).unwrap())
        })
        .collect()
}

/// The Records hold each committed one once, and nothing but those and the
/// last answer's; each Dataset numbers its Records from 1 with no gaps, in
/// the order the model committed them.
fn check(stored: &[Stored], model: &Model) {
    let records: Vec<Value> = stored.iter().map(|(_, _, data)| data.clone()).collect();
    let in_commit_order: Vec<&Value> = model.committed.iter().chain(&model.last_answer).collect();
    for dataset in ["pages", "links"] {
        let numbered: Vec<(i64, &Value)> = stored
            .iter()
            .filter(|(of, _, _)| of == dataset)
            .map(|(_, number, data)| (*number, data))
            .collect();
        let expected: Vec<(i64, &Value)> = in_commit_order
            .iter()
            .filter(|record| dataset_of(record) == dataset)
            .take(numbered.len())
            .enumerate()
            .map(|(index, record)| (i64::try_from(index).unwrap() + 1, *record))
            .collect();
        assert_eq!(
            numbered, expected,
            "`{dataset}` does not number its Records from 1 in commit order"
        );
    }
    let records = &records;
    let mut seen = BTreeMap::new();
    for record in records {
        *seen.entry(record.to_string()).or_insert(0) += 1;
    }
    assert!(
        seen.values().all(|count| *count == 1),
        "a Record is visible twice: {records:?}"
    );
    for record in model.sure() {
        assert!(
            records.contains(record),
            "committed {record} is gone: {records:?}"
        );
    }
    for record in records {
        assert!(
            model.committed.contains(record) || model.last_answer.contains(record),
            "{record} belongs to no committed attempt: {records:?}"
        );
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Model;

    fn init_test(model: &Model) -> Self {
        let env = TestEnv::in_memory();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("delivery-model-{folder}"), env.seed()).unwrap();
        let runtime = goodfetch_test_support::runtime();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (reader, writer) = runtime.block_on(async {
            let dependencies = dependencies(
                &env,
                RunStateOptions {
                    vfs: Some(vfs.name().to_owned()),
                    ..RunStateOptions::default()
                },
            );
            configure(&env, ONE_AT_A_TIME);
            attach(&owner, dependencies).await
        });
        let mut subject = Self {
            runtime,
            env,
            vfs,
            owner,
            lines: BufReader::new(reader).lines(),
            writer,
            delivered: 0,
            finished: false,
            aborted: false,
            silent: Vec::new(),
        };
        subject.hello();
        let seeds: Vec<Value> = (1..=model.items.len())
            .map(|page| {
                let url = if model.items[page - 1].panics {
                    format!("https://shop.test{PANICS}?page={page}")
                } else {
                    format!("https://shop.test/{page}")
                };
                json!({ "url": url, "label": "page" })
            })
            .collect();
        let level = if model.batching { "batch" } else { "full" };
        let params = as_sent(
            "crawl.start",
            json!({
                "run_id": "model",
                "seeds": seeds,
                "datasets": [{ "name": "pages" }, { "name": "links" }],
                "settings": {
                    "run_state": { "durability": level, "memory": { "budget": model.budget } },
                },
            }),
        );
        subject.send(
            &json!({ "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": params }),
        );
        assert_eq!(subject.next()["result"]["started_as"], "new");
        subject
    }

    fn apply(mut subject: Self, model: &Model, transition: Transition) -> Self {
        if subject.finished {
            return subject;
        }
        match transition {
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.stop();
                let connection = after_crash(&subject.vfs, &subject.path()).unwrap();
                check(&stored_records(&connection), model);
                drop::<Connection>(connection);
                // Another engine takes the interrupted Run's writer role
                // to abort it.
                subject.restart();
                let report = subject.call("abort", "run.abort");
                let reported = report["result"]["lifecycle"].as_str().unwrap();
                assert!(
                    model
                        .after_a_crash_and_an_abort()
                        .iter()
                        .any(|lifecycle| lifecycle.as_str() == reported),
                    "{report}"
                );
                check(&subject.records(), model);
                subject.stop();
                subject.finished = true;
            }
            Transition::Abort => {
                if subject.aborted {
                    return subject;
                }
                subject.aborted = true;
                let report = subject.call("abort", "run.abort");
                assert_eq!(
                    (
                        &report["result"]["lifecycle"],
                        &report["result"]["stop_reason"]
                    ),
                    (
                        &json!(model.lifecycle.as_str()),
                        &json!({ "kind": "abort", "by": "client" })
                    ),
                    "{report}"
                );
            }
            Transition::Deliver(behaviour) => {
                if model.delivered == subject.delivered {
                    // Every work item is done: the engine asks nothing more.
                    return subject;
                }
                subject.delivered = model.delivered;
                let request = next_request(&mut subject);
                let index: usize = request["params"]["crawl_request"]["id"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .map(|position: usize| position - 1)
                    .expect("a work item's ID is its position");
                let attempt =
                    u32::try_from(request["params"]["attempt"].as_u64().unwrap()).unwrap();
                assert!(
                    model.items[..index]
                        .iter()
                        .all(|item| item.done || item.panics)
                        && model.items[index].attempts == attempt,
                    "the engine asked for work item {} attempt {attempt}, which the policy does not give: {model:?}",
                    index + 1
                );
                let id = request["id"].clone();
                match behaviour {
                    Behaviour::Records(count) => {
                        let records = pushed((0..count).map(|number| record(index, attempt, number)));
                        subject.send(&json!({ "jsonrpc": "2.0", "id": id, "result": { "records": records } }));
                    }
                    Behaviour::FailsRetryably => subject.send(&json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32003, "message": "retry", "data": { "code": "retry_requested", "retryable": true } },
                    })),
                    Behaviour::FailsItself => subject.send(&json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32001, "message": "a bug", "data": { "code": "handler_failed", "retryable": true } },
                    })),
                    Behaviour::Silent => subject.silent.push((id, Vec::new())),
                    Behaviour::Late(count) => {
                        let records = pushed((0..count).map(|number| record(index, attempt, number)));
                        subject.silent.push((id, records));
                    }
                    Behaviour::Unwritten(count) => {
                        subject.refuse_records(index, attempt);
                        let records = pushed((0..count).map(|number| record(index, attempt, number)));
                        subject.send(&json!({ "jsonrpc": "2.0", "id": id, "result": { "records": records } }));
                    }
                }
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, model: &Model) {
        if !subject.finished {
            check(&subject.records(), model);
        }
    }

    fn teardown(mut subject: Self, model: Model) {
        if subject.finished {
            return;
        }
        if subject.aborted {
            // A finish of the aborted Run changes nothing.
            let report = subject.call("finish", "run.finish");
            assert_eq!(
                report["result"]["lifecycle"],
                model.after(&[RunEvent::Finish]).as_str()
            );
            check(&subject.records(), &model);
        } else if model.next().is_none() {
            let report = subject.call("finish", "run.finish");
            assert_eq!(
                report["result"]["lifecycle"],
                model.after(&[RunEvent::Finish]).as_str()
            );
            let mut expected = model.committed.clone();
            expected.extend(model.last_answer.iter().cloned());
            let mut records: Vec<Value> = subject
                .records()
                .into_iter()
                .map(|(_, _, data)| data)
                .collect();
            records.sort_by_key(Value::to_string);
            expected.sort_by_key(Value::to_string);
            assert_eq!(records, expected);
            // Each work item whose attempts panicked was tried as often as
            // the retry rule allows, and failed with `engine_panicked`.
            let panicked: Vec<(String, String, i64, Option<String>)> = subject
                .work_items()
                .into_iter()
                .filter(|(id, ..)| model.items[id.parse::<usize>().unwrap() - 1].panics)
                .collect();
            let tries = i64::from(ONE_AT_A_TIME.retry_max) + 1;
            assert!(
                panicked.iter().all(|(_, status, attempts, code)| {
                    (status.as_str(), *attempts, code.as_deref())
                        == ("failed", tries, Some("engine_panicked"))
                }),
                "{panicked:?}"
            );
        }
        subject.stop();
    }
}

/// Runs an Attached engine on `dependencies` as a task of `owner`, and
/// connects to it.
async fn attach(
    owner: &TaskOwner,
    dependencies: goodfetch_engine::Dependencies,
) -> (
    goodfetch_engine::transport::ConnectionReader,
    goodfetch_engine::transport::ConnectionWriter,
) {
    let (network, dialer) = transport::in_memory();
    let engine = engine();
    owner.spawn(async move {
        engine
            .run_attached(Network::InMemory(network), dependencies)
            .await
            .unwrap();
    });
    dialer.connect().await.unwrap().split()
}

/// The next message, a `crawl_request.handle` request.
fn next_request(subject: &mut Subject) -> Value {
    let message = next_from_engine(subject);
    assert_eq!(message["method"], "crawl_request.handle", "{message}");
    message
}

/// The next message other than a `$/cancel`, which may only cancel a
/// request left unanswered, and which that request's late answer follows.
fn next_from_engine(subject: &mut Subject) -> Value {
    loop {
        let message = subject.next();
        if message["method"] != "$/cancel" {
            return message;
        }
        let id = &message["params"]["id"];
        let position = subject
            .silent
            .iter()
            .position(|(silent, _)| silent == id)
            .unwrap_or_else(|| {
                panic!("the engine cancelled a request it had an answer to: {message}")
            });
        let (id, records) = subject.silent.remove(position);
        if !records.is_empty() {
            subject.send(&json!({ "jsonrpc": "2.0", "id": id, "result": { "records": records } }));
        }
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-41, story:3-26, story:3-151, story:6-268,
    /// setting:run_state.memory.budget
    #[test]
    fn delivery_commits_each_answered_attempt_once_through_failures_lost_leases_and_crashes(
        sequential 1..20 => Subject
    );
}
