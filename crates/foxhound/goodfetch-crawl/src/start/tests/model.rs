//! The model-based test of delivery (ADR 0060; TESTING.md, Model-based
//! tests and crashes in one process): a crawl on real SQLite through the
//! crash VFS, on the test `Clock`, whose handler answers each attempt as
//! the generated script says (Records, a retryable failure, its own
//! failure, or silence until the lease ends), and which may crash or lose
//! power between attempts. A reference model of the Run's policy says which
//! work item and attempt each handler request must be, and which Records
//! each commit holds. After every step the run file holds every Record
//! acknowledged, none twice and none of an attempt that did not commit;
//! after a crash, the file the next process opens passes `PRAGMA
//! integrity_check` (in `after_crash`) and still does. The handler takes
//! one work item at a time, so a handler request means the previous
//! attempt's outcome was committed. 16 cases in the Quick suite and 256 in
//! the Pre-merge suite (`proptest_config`).

use std::collections::BTreeMap;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Network};
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::runtime::Runtime;

use super::{POLICY, dependencies, engine_with};
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
}

#[derive(Clone, Debug)]
enum Transition {
    /// The next handler request is answered so.
    Deliver(Behaviour),
    /// The process crashes or the power goes, between attempts; the case
    /// ends with the file the next process opens.
    Crash(CrashKind),
}

/// One work item as the policy moves it.
#[derive(Clone, Debug, PartialEq)]
struct Item {
    attempts: u32,
    done: bool,
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
}

impl Model {
    /// The work item the next handler request leases, and its attempt.
    fn next(&self) -> Option<(usize, u32)> {
        self.items
            .iter()
            .position(|item| !item.done)
            .map(|index| (index, self.items[index].attempts + 1))
    }
}

fn record(item: usize, attempt: u32, number: u8) -> Value {
    json!({ "item": item + 1, "attempt": attempt, "number": number })
}

impl ReferenceStateMachine for Model {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        (1_usize..4)
            .prop_map(|count| Self {
                items: vec![
                    Item {
                        attempts: 0,
                        done: false,
                    };
                    count
                ],
                delivered: 0,
                committed: Vec::new(),
                last_answer: Vec::new(),
                crashed: false,
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            3 => (0_u8..3).prop_map(|count| Transition::Deliver(Behaviour::Records(count))),
            1 => Just(Transition::Deliver(Behaviour::FailsRetryably)),
            1 => Just(Transition::Deliver(Behaviour::FailsItself)),
            1 => Just(Transition::Deliver(Behaviour::Silent)),
            1 => prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)].prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut model: Self, transition: &Transition) -> Self {
        if model.crashed {
            return model;
        }
        let Transition::Deliver(behaviour) = transition else {
            model.crashed = true;
            return model;
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
            Behaviour::FailsRetryably | Behaviour::Silent => {
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

    /// The Records the run file holds now, read beside the writer.
    fn records(&self) -> Vec<Value> {
        let connection = Connection::open_with_flags_and_vfs(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            self.vfs.name(),
        )
        .unwrap();
        stored_records(&connection)
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

fn stored_records(connection: &Connection) -> Vec<Value> {
    let mut statement = connection
        .prepare("SELECT data FROM records ORDER BY dataset, number")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect()
}

/// The Records hold each committed one once, and nothing but those and the
/// last answer's.
fn check(records: &[Value], model: &Model) {
    let mut seen = BTreeMap::new();
    for record in records {
        *seen.entry(record.to_string()).or_insert(0) += 1;
    }
    assert!(
        seen.values().all(|count| *count == 1),
        "a Record is visible twice: {records:?}"
    );
    for record in &model.committed {
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
        let env = TestEnv::new();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("delivery-model-{folder}"), env.seed()).unwrap();
        let runtime = goodfetch_test_support::runtime();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let dependencies = dependencies(
            &env,
            RunStateOptions {
                vfs: Some(vfs.name().to_owned()),
            },
        );
        let (reader, writer) = runtime.block_on(async {
            let (network, dialer) = transport::in_memory();
            let engine = engine_with(ONE_AT_A_TIME);
            owner.spawn(async move {
                engine
                    .run_attached(Network::InMemory(network), dependencies)
                    .await
                    .unwrap();
            });
            dialer.connect().await.unwrap().split()
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
        };
        subject.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": { "protocol_version": "0.1", "surface": "rust", "needs": "foxhound", "product_version": "1.4.0" },
        }));
        assert_eq!(subject.next()["result"]["product"], "foxhound");
        let seeds: Vec<Value> = (1..=model.items.len())
            .map(|page| json!({ "url": format!("https://shop.test/{page}"), "label": "page" }))
            .collect();
        subject.send(&json!({
            "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
            "params": { "run_id": "model", "seeds": seeds },
        }));
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
                subject.finished = true;
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
                    model.items[..index].iter().all(|item| item.done)
                        && model.items[index].attempts == attempt,
                    "the engine asked for work item {} attempt {attempt}, which the policy does not give: {model:?}",
                    index + 1
                );
                let id = request["id"].clone();
                match behaviour {
                    Behaviour::Records(count) => {
                        let records: Vec<Value> = (0..count)
                            .map(|number| json!({ "dataset": "pages", "data": record(index, attempt, number) }))
                            .collect();
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
                    Behaviour::Silent => {}
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
        if model.next().is_none() {
            subject.send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "model" } }));
            let report = loop {
                let message = subject.next();
                if message["id"] == "finish" {
                    break message;
                }
            };
            assert_eq!(report["result"]["lifecycle"], "finished");
            let mut expected = model.committed.clone();
            expected.extend(model.last_answer.iter().cloned());
            let mut records = subject.records();
            records.sort_by_key(Value::to_string);
            expected.sort_by_key(Value::to_string);
            assert_eq!(records, expected);
        }
        subject.stop();
    }
}

/// The next message, a `crawl_request.handle` request.
fn next_request(subject: &mut Subject) -> Value {
    let message = subject.next();
    assert_eq!(message["method"], "crawl_request.handle", "{message}");
    message
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-41, story:3-26
    #[test]
    fn delivery_commits_each_answered_attempt_once_through_failures_lost_leases_and_crashes(
        sequential 1..20 => Subject
    );
}
