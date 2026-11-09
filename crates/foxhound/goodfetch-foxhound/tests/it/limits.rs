//! Limits across runs held by every engine of the user (spec #213's
//! multi-process tests; ADRs 0041, 0060 and 0098): under one home, a Daemon
//! and two Attached engines of Foxhound's executable share one Usage ledger
//! and one install Limit. The first Attached engine sets the Limit, starts a
//! crawl whose handler never answers, and is killed (`kill -9`,
//! `TerminateProcess` on Windows) at a moment drawn from the seed among its
//! `crawl_request.handle` requests, its Slices still held. The second
//! Attached engine, which reclaims them when it opens the ledger, and a
//! crawl on the Daemon then crawl at once until the Limit stops both at
//! their next safe point. What the ledger charged and holds never exceeds
//! the Limit, the killed run is charged what it journaled, and no run
//! committed more than was charged. A crawl needs Foxhound, so every engine
//! is Foxhound's (ADR 0094). Fixed seeds run in the Pre-merge suite; the
//! case seeded by `GOODFETCH_TEST_SEED` takes new seeds in the nightly-only
//! job.
#![expect(
    clippy::disallowed_methods,
    reason = "each Client is a thread of the test's own, and the test reads the files the engines wrote"
)]

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    DAEMON_DEADLINE, Daemon, DaemonEndpoint, Moment, ObservationLog, Operation, Record, TestEnv,
    kill_at, limits, record_command,
};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached;
use crate::governor::in_flight_whatever_the_machine;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The install Limit on requests.
const LIMIT: i64 = 40;

/// How many seeds the killed engine's crawl has.
const VICTIM_SEEDS: u64 = 10;

/// How many seeds each surviving crawl has: together more than the Limit.
const SEEDS: u64 = 60;

/// Among how many of the killed engine's first `crawl_request.handle`
/// requests the kill's moment is drawn: its handler answers none, so the
/// home's governors keep this many in flight whatever the machine.
const OCCURRENCES: u32 = 5;

fn seeds(site: &str, count: u64) -> Value {
    (1..=count)
        .map(|n| json!({ "url": format!("https://{site}.test/{n}"), "label": "page" }))
        .collect()
}

fn line(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// Starts an Attached engine that sets the install Limit and starts a
/// crawl whose handler never answers, and kills it at `moment`.
fn killed_at(env: &TestEnv, moment: &Moment, log: &mut ObservationLog<Value>) {
    env.log_level("trace");
    let mut command = env.process_command(FOXHOUND);
    command.arg("attached-engine").stderr(Stdio::null());
    let victim = env.spawn(&mut command).unwrap();
    {
        let mut child = victim.child();
        let stdin = child.stdin.as_mut().unwrap();
        for message in [
            Hello::needing(Product::Foxhound).id("hello").message(),
            json!({
                "jsonrpc": "2.0", "id": "limit", "method": "usage.set",
                "params": { "scope": "install", "usage_kind": "requests", "amount": LIMIT },
            }),
            json!({
                "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
                "params": { "run_id": "victim", "seeds": seeds("victim", VICTIM_SEEDS), "datasets": [{ "name": "pages" }] },
            }),
        ] {
            stdin.write_all(&line(&message)).unwrap();
        }
        stdin.flush().unwrap();
    }
    kill_at(&victim, moment, log).unwrap();
    record_command(env, &["attached-engine"], None).unwrap();
}

/// Answers a handler request with one Record naming its URL.
fn handled(params: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": params["crawl_request"]["url"] } }] } })
}

/// Crawls `site` on the Attached engine `engine` until its Run ends, and
/// answers its Run report.
fn crawled_attached(mut engine: Attached, site: &str) -> Value {
    engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": site, "seeds": seeds(site, SEEDS), "datasets": [{ "name": "pages" }] }),
        &mut |_| None,
    );
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": site }),
        &mut |params| Some(handled(params)),
    );
    assert!(engine.finished(), "the Attached engine failed");
    report["result"].clone()
}

/// Crawls `site` on the Daemon until its Run ends, and answers its Run
/// report.
fn crawled_on_the_daemon(endpoint: &DaemonEndpoint, token: &str, site: &str) -> Value {
    let mut client = DaemonClient::opened(
        endpoint,
        &Hello::needing(Product::Foxhound).id("hello").token(token),
    );
    let mut handler = |params: &Value| Some(handled(params));
    client.call_handling(
        "start",
        "crawl.start",
        &json!({ "run_id": site, "seeds": seeds(site, SEEDS), "datasets": [{ "name": "pages" }] }),
        &mut handler,
    );
    client.call_handling(
        "finish",
        "run.finish",
        &json!({ "run_id": site }),
        &mut handler,
    )["result"]
        .clone()
}

/// What the ledger file the engines wrote holds of the install Limit: each
/// Slice's size, what it charged once closed and its state, in the order
/// they were taken.
fn slices(env: &TestEnv) -> Vec<(i64, Option<i64>, String)> {
    let ledger = Connection::open(env.home().join("usage-ledger.sqlite")).unwrap();
    let mut statement = ledger
        .prepare("SELECT size, charged, state FROM slices ORDER BY slice_id")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// What the Run `run_id` journaled of its Slices, and the Records it
/// committed.
fn journaled_and_committed(env: &TestEnv, run_id: &str) -> (i64, i64) {
    let file = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let journaled: i64 = file
        .query_row("SELECT COALESCE(SUM(used), 0) FROM usage", [], |row| {
            row.get(0)
        })
        .unwrap();
    let committed: i64 = file
        .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
        .unwrap();
    (journaled, committed)
}

/// Covers: story:3-67, story:3-69, story:3-71, story:3-72
#[rstest]
#[case::seed_7(Some(7))]
#[case::seed_11(Some(11))]
#[case::the_tests_seed(None)]
fn engines_of_one_user_never_charge_past_a_limit_through_a_killed_engine(
    #[case] seed: Option<u64>,
) {
    let env = TestEnv::new();
    env.user_configuration(&in_flight_whatever_the_machine(OCCURRENCES));
    let daemon = Daemon::start_program(&env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap();
    let (endpoint, token) = (
        daemon.endpoint().clone(),
        daemon.description().token.clone(),
    );
    let moment = Moment::drawn(
        seed.unwrap_or(env.seed()),
        &["crawl_request.handle"],
        OCCURRENCES,
    );
    let mut log = ObservationLog::<Value>::for_test(&env);
    killed_at(&env, &moment, &mut log);

    let (attached, on_the_daemon) = std::thread::scope(|scope| {
        let on_the_daemon = scope.spawn(|| crawled_on_the_daemon(&endpoint, &token, "daemon"));
        let attached = crawled_attached(attached(&env), "attached");
        (attached, on_the_daemon.join().unwrap())
    });
    daemon.stop(&env).unwrap();

    let slices = slices(&env);
    log.push(Record::new(
        "ledger",
        Operation::Limit {
            scope: "install/requests".to_owned(),
            usage: u64::try_from(LIMIT).unwrap(),
        },
    ));
    for (size, charged, state) in &slices {
        log.push(Record::new(
            "ledger",
            Operation::Reserve {
                scope: "install/requests".to_owned(),
                usage: u64::try_from(*size).unwrap(),
            },
        ));
        if state != "held" {
            log.push(Record::new(
                "ledger",
                Operation::Settle {
                    scope: "install/requests".to_owned(),
                    reserved: u64::try_from(*size).unwrap(),
                    charged: u64::try_from(charged.unwrap_or(0)).unwrap(),
                },
            ));
        }
    }
    let charged: i64 = slices.iter().filter_map(|(_, charged, _)| *charged).sum();
    let (victim, _) = journaled_and_committed(&env, "victim");
    let (attached_used, attached_committed) = journaled_and_committed(&env, "attached");
    let (daemon_used, daemon_committed) = journaled_and_committed(&env, "daemon");
    assert_eq!(limits(&log, 0), Ok(()));
    assert_eq!(
        (
            (&attached["lifecycle"], &attached["stop_reason"]["kind"]),
            (
                &on_the_daemon["lifecycle"],
                &on_the_daemon["stop_reason"]["kind"]
            ),
            slices
                .iter()
                .filter(|(_, _, state)| state == "held")
                .count(),
            charged,
            attached_committed <= attached_used && daemon_committed <= daemon_used,
        ),
        (
            (&json!("stopped"), &json!("limit")),
            (&json!("stopped"), &json!("limit")),
            0,
            victim + attached_used + daemon_used,
            true,
        ),
        "{slices:?}"
    );
}
