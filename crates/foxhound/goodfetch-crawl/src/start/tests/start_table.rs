//! The model-based test of the start table (ADRs 0060 and 0102; TESTING.md,
//! Model-based tests and crashes in one process). One Run ID is started
//! again and again as a keep-alive crawl on real SQLite through the crash
//! VFS, on the test `Clock`. The crawl's seeds are its inputs, and
//! `crawl.scope.strategy` is a key fixed for the Run's life (ADR 0120). The
//! Run is paused, finished, its run file damaged, and its engine stopped
//! cleanly, crashed or cut from power and started again.
//!
//! A reference model of the Run says what each `crawl.start` answers:
//! - `new` for a Run with no state;
//! - `run_conflict` for other seeds, then `setting_not_applicable` for
//!   another value of the fixed key, which the run file keeps as the Run
//!   began;
//! - then `run_in_use` for a Run live in this engine, `resumed` for an
//!   interrupted one and `ended` for a finished one;
//! - `run_state_unavailable` for a Run whose state cannot be read.
//!
//! The Run is at `full` (`run_state.durability`), where a crash of either
//! kind leaves a live Run interrupted, as a clean stop does. `off` is not
//! drawn here. After the crash VFS crashes, the engine in this process
//! goes on to its own stop, and it may reopen the run file and end the Run
//! cleanly, which no crashed process does. So the discard of a run file
//! that a crash left at `off` (ADR 0138) is the Run state model's to test,
//! which closes its writer at the crash (`run_state::tests::model`).
//!
//! 16 cases in the Quick suite and 256 in the Pre-merge suite
//! (`proptest_config`). Each case's folder lies in memory
//! (`TestEnv::in_memory`).

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::runtime::Runtime;

use super::model::attach;
use super::{as_sent, dependencies, seeds};

/// The Run ID every step names.
const RUN_ID: &str = "table";

/// What a start declares of the Run: how many seeds, its inputs, and
/// whether it sets `crawl.scope.strategy` to `same_domain`, a key fixed
/// for the Run's life, or leaves its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Declared {
    seeds: usize,
    same_domain: bool,
}

/// The Run as the reference model knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    /// No state: never started.
    Absent,
    /// Live in this engine, as it began.
    Live(Declared),
    /// Interrupted, which a start resumes.
    Interrupted(Declared),
    /// Finished, which a start answers as it ended.
    Finished(Declared),
    /// Its run file cannot be read.
    Damaged,
}

#[derive(Clone, Copy, Debug)]
enum Transition {
    /// A `crawl.start` under the Run ID that declares this.
    Start(Declared),
    /// A `run.pause` of the live Run.
    Pause,
    /// A `run.finish` of the live Run.
    Finish,
    /// The engine stops cleanly, and another one starts.
    Restart,
    /// The engine crashes, and another one starts.
    Crash(CrashKind),
    /// The run file of the Run, which no engine runs, loses its `run`
    /// table.
    Damage,
}

#[derive(Clone, Debug)]
struct Model {
    run: Run,
    /// What the last start answered, as the model says before it applied
    /// it: the test reads the model after each transition.
    answered: &'static str,
}

impl Model {
    /// What a start that declares `declared` answers: its `started_as`, or
    /// its error's code.
    fn answer(&self, declared: Declared) -> &'static str {
        let began = match self.run {
            Run::Absent => return "new",
            Run::Damaged => return "run_state_unavailable",
            Run::Live(began) | Run::Interrupted(began) | Run::Finished(began) => began,
        };
        if began.seeds != declared.seeds {
            "run_conflict"
        } else if began.same_domain != declared.same_domain {
            "setting_not_applicable"
        } else {
            match self.run {
                Run::Live(_) => "run_in_use",
                Run::Interrupted(_) => "resumed",
                _ => "ended",
            }
        }
    }
}

fn declared() -> impl Strategy<Value = Declared> {
    (1_usize..=2, any::<bool>()).prop_map(|(seeds, same_domain)| Declared { seeds, same_domain })
}

impl ReferenceStateMachine for Model {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self {
            run: Run::Absent,
            answered: "",
        })
        .boxed()
    }

    fn transitions(state: &Self) -> BoxedStrategy<Transition> {
        let crash = prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)];
        match state.run {
            Run::Live(_) => prop_oneof![
                4 => declared().prop_map(Transition::Start),
                1 => Just(Transition::Pause),
                1 => Just(Transition::Finish),
                1 => Just(Transition::Restart),
                1 => crash.prop_map(Transition::Crash),
            ]
            .boxed(),
            Run::Interrupted(_) | Run::Finished(_) => prop_oneof![
                4 => declared().prop_map(Transition::Start),
                1 => Just(Transition::Restart),
                1 => crash.prop_map(Transition::Crash),
                1 => Just(Transition::Damage),
            ]
            .boxed(),
            Run::Absent | Run::Damaged => prop_oneof![
                4 => declared().prop_map(Transition::Start),
                1 => Just(Transition::Restart),
                1 => crash.prop_map(Transition::Crash),
            ]
            .boxed(),
        }
    }

    fn preconditions(state: &Self, transition: &Transition) -> bool {
        match transition {
            Transition::Pause | Transition::Finish => matches!(state.run, Run::Live(_)),
            Transition::Damage => matches!(state.run, Run::Interrupted(_) | Run::Finished(_)),
            Transition::Start(_) | Transition::Restart | Transition::Crash(_) => true,
        }
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        if let Transition::Start(declared) = *transition {
            state.answered = state.answer(declared);
        }
        state.run = match (*transition, state.run) {
            (Transition::Start(declared), Run::Absent) => Run::Live(declared),
            (Transition::Start(_), Run::Interrupted(began)) if state.answered == "resumed" => {
                Run::Live(began)
            }
            (Transition::Pause | Transition::Restart, Run::Live(began)) => Run::Interrupted(began),
            (Transition::Finish, Run::Live(began)) => Run::Finished(began),
            (Transition::Crash(_), Run::Live(began)) => Run::Interrupted(began),
            (Transition::Damage, _) => Run::Damaged,
            (_, run) => run,
        };
        state
    }
}

/// The engine under test on the crash VFS, and its Client.
struct Subject {
    runtime: Runtime,
    env: TestEnv,
    vfs: CrashVfs,
    owner: TaskOwner,
    lines: Lines<BufReader<goodfetch_engine::transport::ConnectionReader>>,
    writer: goodfetch_engine::transport::ConnectionWriter,
    calls: u64,
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

    /// Starts an engine in the home on the crash VFS and says `hello`.
    fn attach(&mut self) {
        let options = RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
            ..RunStateOptions::default()
        };
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
        let (env, owner) = (&self.env, &self.owner);
        let (reader, writer) = self
            .runtime
            .block_on(async { attach(owner, dependencies(env, options)).await });
        self.lines = BufReader::new(reader).lines();
        self.writer = writer;
        self.hello();
    }

    fn hello(&mut self) {
        self.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": { "protocol_version": "0.1", "surface": "rust", "needs": "foxhound", "product_version": "1.4.0" },
        }));
        assert_eq!(self.next()["result"]["product"], "foxhound");
    }

    /// Calls `method` with `params` and returns its reply, answering each
    /// handler request on the way with no Record.
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.calls += 1;
        let id = format!("call-{}", self.calls);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next();
            if message["id"] == id.as_str() {
                return message;
            }
            if message["method"] == "crawl_request.handle" {
                let request = message["id"].clone();
                self.send(&json!({ "jsonrpc": "2.0", "id": request, "result": { "records": [] } }));
            }
        }
    }

    /// What a `crawl.start` that declares `declared` answered: its
    /// `started_as`, or its error's code.
    fn start(&mut self, declared: Declared) -> String {
        let mut crawl = json!({ "keep_alive": true });
        if declared.same_domain {
            crawl["scope"] = json!({ "strategy": "same_domain" });
        }
        let params = as_sent(
            "crawl.start",
            json!({
                "run_id": RUN_ID,
                "seeds": seeds(declared.seeds),
                "labels": { "page": {} },
                "settings": { "crawl": crawl, "run_state": { "durability": "full" } },
            }),
        );
        let answer = self.call("crawl.start", params);
        let outcome = answer["result"]["started_as"]
            .as_str()
            .or_else(|| answer["error"]["data"]["code"].as_str());
        outcome
            .unwrap_or_else(|| panic!("a start answered neither: {answer}"))
            .to_owned()
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

    /// The Run's run file, as the crash VFS names it.
    fn path(&self) -> String {
        super::run_folder(&self.env, RUN_ID)
            .join("run.sqlite")
            .to_string_lossy()
            .into_owned()
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Model;

    fn init_test(_: &Model) -> Self {
        let env = TestEnv::in_memory();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("start-table-{folder}"), env.seed()).unwrap();
        let runtime = goodfetch_test_support::runtime();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let options = RunStateOptions {
            vfs: Some(vfs.name().to_owned()),
            ..RunStateOptions::default()
        };
        let (reader, writer) =
            runtime.block_on(async { attach(&owner, dependencies(&env, options)).await });
        let mut subject = Self {
            runtime,
            env,
            vfs,
            owner,
            lines: BufReader::new(reader).lines(),
            writer,
            calls: 0,
        };
        subject.hello();
        subject
    }

    fn apply(mut subject: Self, model: &Model, transition: Transition) -> Self {
        match transition {
            Transition::Start(declared) => {
                let answer = subject.start(declared);
                assert_eq!(answer, model.answered, "{transition:?}, to {model:?}");
            }
            Transition::Pause => {
                let paused = subject.call("run.pause", json!({ "run_id": RUN_ID }));
                assert_eq!(paused["result"]["lifecycle"], "stopped", "{paused}");
            }
            Transition::Finish => {
                let finished = subject.call("run.finish", json!({ "run_id": RUN_ID }));
                assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
            }
            Transition::Restart => {
                subject.stop();
                subject.attach();
            }
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.stop();
                subject.attach();
            }
            Transition::Damage => {
                Connection::open_with_flags_and_vfs(
                    subject.path(),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                    subject.vfs.name(),
                )
                .and_then(|connection| connection.execute_batch("DROP TABLE run"))
                .unwrap();
            }
        }
        subject
    }

    fn check_invariants(_: &Self, _: &Model) {}

    fn teardown(mut subject: Self, _: Model) {
        subject.stop();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-177, error:run_conflict, error:run_in_use,
    /// error:setting_not_applicable, error:run_state_unavailable,
    /// operation:crawl.start, setting:crawl.scope.strategy
    #[test]
    fn a_start_answers_as_the_start_table_says_through_pauses_finishes_crashes_and_power_cuts(
        sequential 1..20 => Subject
    );
}
