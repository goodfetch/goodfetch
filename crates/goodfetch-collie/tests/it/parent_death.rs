//! An Attached engine dies with its Client on Collie's executable (spec
//! #213's multi-process tests; ADRs 0024, 0060 and 0094): the Client that
//! started the engine is killed (`kill -9`, `TerminateProcess` on Windows)
//! at a moment drawn from the test's seed among the `$/events` it observed,
//! while its stdin stays open, as when a forked child still holds it, and
//! the engine still ends: the kernel kills it on Linux (`PR_SET_PDEATHSIG`)
//! and on Windows (a kill-on-close Job Object), and its kqueue `NOTE_EXIT`
//! watch stops it on macOS. Each Run's writer lock is then free. Fixed
//! seeds run in the Pre-merge suite and on every OS in the Release suite;
//! the case seeded by `GOODFETCH_TEST_SEED` takes new seeds in the
//! nightly-only job.
#![expect(
    clippy::disallowed_methods,
    reason = "the reader is a thread of the test's own, and the test tests the Runs' writer locks itself"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{ChildStdin, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use goodfetch_engine::runs::{Writer, test_writer};
use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::Hello;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{Moment, ObservationLog, Operation, Record, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// How many Runs the Client starts, each one `$/events` notification, one
/// at a time.
const RUNS: u32 = 4;

/// How long the test waits for each message, and for the engine's end.
const DEADLINE: Duration = Duration::from_secs(10);

/// The Client's stand-in: a process that starts the engine as its child
/// and waits for it, while the engine's stdin and stdout are the test's
/// own pipes, which outlive it.
#[cfg(unix)]
fn client(env: &TestEnv) -> std::process::Command {
    let mut command = env.process_command("/bin/sh");
    command
        .args(["-c", "\"$@\"; exit $?", "sh", COLLIE, "attached-engine"])
        .stderr(Stdio::null());
    command
}

#[cfg(windows)]
fn client(env: &TestEnv) -> std::process::Command {
    let mut command = env.process_command("cmd.exe");
    command
        .args(["/d", "/c", COLLIE, "attached-engine"])
        .stderr(Stdio::null());
    command
}

fn send(stdin: &mut ChildStdin, message: &Value) {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    stdin.write_all(&line).unwrap();
    stdin.flush().unwrap();
}

/// Covers: story:3-14
#[rstest]
#[case::seed_5(Some(5))]
#[case::seed_9(Some(9))]
#[case::the_tests_seed(None)]
fn an_attached_engine_ends_when_its_client_is_killed_without_closing_its_stdin(
    #[case] seed: Option<u64>,
) {
    let env = TestEnv::new();
    let moment = Moment::drawn(seed.unwrap_or(env.seed()), &["$/events"], RUNS);
    let mut log = ObservationLog::<Value>::for_test(&env);
    let client = env.spawn(&mut client(&env)).unwrap();
    let mut stdin = client.child().stdin.take().unwrap();
    let stdout = client.child().stdout.take().unwrap();
    let (lines, messages) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let next = || -> Option<Value> {
        match messages.recv_timeout(DEADLINE) {
            Ok(line) => Some(serde_json::from_str(&line.unwrap()).unwrap()),
            Err(RecvTimeoutError::Disconnected) => None,
            Err(RecvTimeoutError::Timeout) => panic!("the engine sent nothing within {DEADLINE:?}"),
        }
    };
    send(
        &mut stdin,
        &Hello::needing(Product::Collie).id("hello").message(),
    );
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": RUNS } }),
    );
    let mut observed = 0;
    let mut started = Vec::new();
    'runs: for index in 1..=RUNS {
        let run_id = format!("run-{index}");
        send(
            &mut stdin,
            &json!({ "jsonrpc": "2.0", "id": index, "method": "run.start", "params": { "run_id": run_id } }),
        );
        started.push(run_id);
        let (mut answered, mut evented) = (false, false);
        while !(answered && evented) {
            let message = next().expect("the engine ended before the moment");
            if message["id"] == json!(index) {
                answered = true;
            } else if message["method"] == "$/events" {
                evented = true;
                observed += 1;
                if observed == moment.occurrence {
                    break 'runs;
                }
            }
        }
    }

    client.child().kill().unwrap();
    client.child().wait().unwrap();
    log.push(Record::new(
        "client",
        Operation::Other(
            json!({ "killed_at": { "event": moment.event, "occurrence": moment.occurrence } }),
        ),
    ));
    // The engine's stdout closes once it ended, since the killed Client's
    // copy closed with it.
    loop {
        match messages.recv_timeout(DEADLINE) {
            Ok(_) => {}
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                panic!("the engine outlived its killed Client by {DEADLINE:?}")
            }
        }
    }

    let runs = HomeLayout::of(env.home()).runs();
    let writers: Vec<Writer> = started
        .iter()
        .map(|run_id| test_writer(&runs.join(run_id)).unwrap())
        .collect();
    drop(stdin);
    assert_eq!(
        writers,
        vec![Writer::None; started.len()],
        "the killed Client's engine still writes its Runs"
    );
}
