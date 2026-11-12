//! A reader's test of the writer lock racing a writer's takeover across
//! processes (spec #213's multi-process tests; ADRs 0031, 0037's
//! consequence and 0060): an Attached engine runs a crawl and is killed
//! (`kill -9`, `TerminateProcess` on Windows) at a moment drawn from the
//! seed among its `crawl_request.handle` requests; then, while a reader
//! tests the Run's writer lock again and again, as the Console does, another
//! engine aborts the Run, taking its writer role at the next run epoch. The
//! writer never fails because of the reader. A crawl needs Foxhound, so
//! both engines are Foxhound's (ADR 0094). Fixed seeds run in the
//! Pre-merge suite and on every OS in the Release suite; the case seeded by
//! `GOODFETCH_TEST_SEED` takes new seeds in the nightly-only job.
#![expect(
    clippy::disallowed_methods,
    reason = "the reader is a thread of the test's own, and the test reads the run file itself"
)]

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_engine::run_state::run_row;
use goodfetch_engine::runs::{Writer, test_writer};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::RUN_FILE;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_client::Hello;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    Moment, ObservationLog, Operation, Process, Record, StrictStdout, TestEnv, kill_at,
    record_command,
};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::governor::in_flight_whatever_the_machine;
use crate::site::Site;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// How many work items the crawl has, each one `crawl_request.handle`
/// request the engine sends at once, the handler never answering: the
/// home's governor keeps this many in flight whatever the machine.
const WORK_ITEMS: u32 = 5;

fn hello() -> Value {
    Hello::needing(Product::Foxhound).id("hello").message()
}

/// An Attached engine logging at `trace`, into the home's log file as
/// test support asks and on stderr: piped when `read`, as `StrictStdout`
/// wants, and otherwise nowhere, since nothing reads it before the kill.
fn engine(env: &TestEnv, read: bool) -> Process {
    env.log_level("trace");
    let mut command = env.process_command(FOXHOUND);
    command.arg("attached-engine");
    if read {
        command.stderr(Stdio::piped());
    } else {
        command.stderr(Stdio::null());
    }
    env.spawn(&mut command).unwrap()
}

fn send(process: &Process, messages: &[Value]) {
    let mut child = process.child();
    let stdin = child.stdin.as_mut().unwrap();
    for message in messages {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).unwrap();
    }
    stdin.flush().unwrap();
}

/// Starts a crawl of `WORK_ITEMS` seeds as the Run `run_id` and kills its
/// engine at `moment`.
fn killed_at(
    (env, site): (&TestEnv, &Site),
    run_id: &str,
    moment: &Moment,
    log: &mut ObservationLog<Value>,
) {
    let victim = engine(env, false);
    let seeds: Vec<Value> = (1..=WORK_ITEMS)
        .map(|n| json!({ "url": site.page(&format!("/{n}")), "label": "page" }))
        .collect();
    send(
        &victim,
        &[
            hello(),
            json!({ "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": { "run_id": run_id, "seeds": seeds, "settings": site.settings() } }),
        ],
    );
    kill_at(&victim, moment, log).unwrap();
    record_command(env, &["attached-engine"], None).unwrap();
}

/// Aborts `run_id` through a new Attached engine, and answers its reply.
fn aborted(env: &TestEnv, run_id: &str) -> Value {
    let writer = engine(env, true);
    let mut stdout = StrictStdout::of(&writer).unwrap();
    send(
        &writer,
        &[
            hello(),
            json!({ "jsonrpc": "2.0", "id": "abort", "method": "run.abort", "params": { "run_id": run_id } }),
        ],
    );
    let answer = loop {
        let message = stdout.try_next().unwrap().expect("the engine closed first");
        if message["id"] == "abort" {
            break message;
        }
    };
    drop(writer.child().stdin.take());
    while stdout.try_next().unwrap().is_some() {}
    let status = writer.child().wait().unwrap();
    record_command(env, &["attached-engine"], status.code()).unwrap();
    answer
}

/// Tests the writer lock of `folder` until `done` says so, a fraction of
/// a millisecond apart, and answers how many tests it made and how many
/// found the writer live.
fn reader(folder: &Path, done: &mpsc::Receiver<()>) -> (u64, u64) {
    let (mut tests, mut live) = (0, 0);
    while done.try_recv() == Err(mpsc::TryRecvError::Empty) {
        if test_writer(folder).unwrap() == Writer::Live {
            live += 1;
        }
        tests += 1;
        std::thread::sleep(Duration::from_micros(200));
    }
    (tests, live)
}

/// Covers: story:3-46, operation:run.abort, operation:crawl.start
#[rstest]
#[case::seed_7(Some(7))]
#[case::seed_11(Some(11))]
#[case::the_tests_seed(None)]
fn a_readers_test_racing_a_takeover_of_a_killed_engines_crawl_never_fails_the_writer(
    #[case] seed: Option<u64>,
) {
    let env = TestEnv::new();
    env.user_configuration(&in_flight_whatever_the_machine(WORK_ITEMS));
    let moment = Moment::drawn(
        seed.unwrap_or(env.seed()),
        &["crawl_request.handle"],
        WORK_ITEMS,
    );
    let mut log = ObservationLog::<Value>::for_test(&env);
    let site = Site::start(&env);
    killed_at((&env, &site), "raced", &moment, &mut log);
    let folder = HomeLayout::of(env.home()).run_folder("raced");
    let (done, until) = mpsc::channel();
    let racing = {
        let folder = folder.clone();
        std::thread::spawn(move || reader(&folder, &until))
    };

    let answer = aborted(&env, "raced");
    done.send(()).unwrap();
    let (tests, live) = racing.join().unwrap();
    log.push(Record::new(
        "reader",
        Operation::Other(json!({ "tests": tests, "live": live })),
    ));
    log.push(Record::new(
        "writer",
        Operation::Other(json!({ "aborted": answer })),
    ));
    let file = Connection::open(folder.join(RUN_FILE)).unwrap();
    let (lifecycle, epoch) = (
        run_row::lifecycle(&file).unwrap(),
        run_row::epoch(&file).unwrap(),
    );
    assert_eq!(
        (
            answer["result"]["lifecycle"].clone(),
            answer["result"]["stop_reason"].clone(),
            lifecycle,
            epoch,
            tests > 0,
        ),
        (
            json!("stopped"),
            json!({ "kind": "abort", "by": "client" }),
            Lifecycle::Stopped,
            2,
            true,
        ),
        "the writer failed: {answer}"
    );
}
