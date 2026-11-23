//! J-005, "Resume a Crawl after the Daemon is killed, with no Record
//! delivered twice", on the `cli` surface (ADRs 0060, 0063, 0075 and
//! 0094): a person starts the Daemon with Foxhound's shipped executable,
//! starts a code-free crawl of the Proving ground's catalog on it, reads its
//! Records as they arrive, kills the Daemon (`kill -9`, `TerminateProcess`
//! on Windows) once some are committed, starts the Daemon again, resumes
//! the crawl by its Run ID and reads its Records after the last one read,
//! until it finishes.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the settings and fields files, read the Records as a process prints them and kill the Daemon"
)]

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, TestEnv, cli as command_line};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;
use crate::journeys::j001::{json_of, wait_until_stopped};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The person's crawl.
pub(crate) const RUN: &str = "catalog";

/// How many Records the person reads before killing the Daemon.
const BEFORE_THE_KILL: usize = 3;

/// The fields each product's Record holds, read from its product page.
const PRODUCT_FIELDS: [&str; 3] = ["id", "name", "price"];

/// The named fields the crawl reads from each page, as `--fields` takes
/// them: each product's ID, name and price from its product page.
pub(crate) fn fields_read() -> Value {
    json!({
        "id": { "locator": { "kind": "css", "selector": "article.product .id" } },
        "name": { "locator": { "kind": "css", "selector": "article.product .name" } },
        "price": {
            "locator": { "kind": "css", "selector": "article.product .price" },
            "attribute": "data-price",
        },
    })
}

/// The settings file of the crawl's `crawl start`: over HTTP, one page at
/// a time, 200 ms apart, so the person acts while most of the catalog is
/// still to crawl.
pub(crate) fn crawl_settings(run: &JourneyRun<impl Sized>) -> PathBuf {
    let proving_ground = run
        .proving_ground()
        .expect("the Journey has a Proving ground");
    let path = run.env().folder().join("settings.json");
    let more = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "concurrency": 1, "delay": 200 } },
    });
    std::fs::write(&path, settings(proving_ground, &more).to_string()).unwrap();
    path
}

/// Starts the person's code-free crawl of the catalog under [`RUN`], on
/// the running Daemon, as `crawl start` does: its pages as Markdown, its
/// links followed within its scope, and each product's fields read. It
/// returns the crawl's settings file, which its resume takes again: a
/// resume takes the settings of its own call, and the Proving ground's
/// hosts are reached through them.
pub(crate) fn start_crawl(run: &JourneyRun<impl Sized>) -> PathBuf {
    let proving_ground = run
        .proving_ground()
        .expect("the Journey has a Proving ground");
    let settings = crawl_settings(run);
    let fields = run.env().folder().join("fields.json");
    std::fs::write(&fields, fields_read().to_string()).unwrap();
    let start = proving_ground.start_url("catalog/1").unwrap();
    let started = command_line(run.env(), run.engine())
        .args(["crawl", "start", &start, "--run-id", RUN])
        .args(["--format", "markdown", "--links", "scope"])
        .args(["--fields", fields.to_str().unwrap()])
        .args(["--settings", settings.to_str().unwrap(), "--json"])
        .json("RunId")
        .unwrap();
    assert_eq!(started.documents[0], RUN, "{}", started.output);
    settings
}

/// Resumes the crawl by its Run ID with `run resume`, given its settings
/// file again, and returns what it printed.
pub(crate) fn resume_crawl(run: &JourneyRun<impl Sized>, settings: &Path) -> Value {
    json_of(
        run.engine(),
        run.env(),
        &[
            "run",
            "resume",
            RUN,
            "--settings",
            settings.to_str().unwrap(),
            "--json",
        ],
    )
}

/// The first `count` Records of the crawl's Dataset, read as they arrive
/// by `run records --follow`, which the person then interrupts.
pub(crate) fn first_records(run: &JourneyRun<impl Sized>, count: usize) -> Vec<Value> {
    let mut command = run.env().process_command(run.engine());
    command
        .args(["run", "records", RUN, "--dataset", "pages", "--follow"])
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let reader = run.env().spawn(&mut command).unwrap();
    let stdout = reader.child().stdout.take().unwrap();
    let records: Vec<Value> = BufReader::new(stdout)
        .lines()
        .take(count)
        .map(|line| serde_json::from_str::<Value>(&line.unwrap()).unwrap()["record"].clone())
        .collect();
    let mut child = reader.child();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(records.len(), count, "{records:?}");
    records
}

/// What `run resume` prints once it resumed the crawl `run` in the same
/// Run: running again, as Foxhound's.
pub(crate) fn resumed_on(run: &str) -> Value {
    json!({ "run_id": run, "started_as": "resumed", "lifecycle": "running", "needs": "foxhound" })
}

/// The ID of the process the home's Daemon runs as, from `daemon status`.
fn daemon_pid(env: &TestEnv, engine: &Path) -> Value {
    let status = command_line(env, engine)
        .args(["daemon", "status", "--json"])
        .json("DaemonStatus")
        .unwrap();
    status.documents[0]["pid"].clone()
}

/// Kills the process `pid` as `kill -9` does, `TerminateProcess` on
/// Windows.
fn kill(pid: &Value) {
    let pid = pid.as_u64().expect("the Daemon runs").to_string();
    #[cfg(unix)]
    let status = std::process::Command::new("kill")
        .args(["-KILL", &pid])
        .status()
        .unwrap();
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid])
        .status()
        .unwrap();
    assert!(status.success(), "the Daemon {pid} was not killed");
}

/// The Records of the crawl's Dataset, each as `run records` reads it.
pub(crate) fn dataset(run: &JourneyRun<impl Sized>) -> Vec<Value> {
    let read = command_line(run.env(), run.engine())
        .args(["run", "records", RUN, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    read.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|read| read["record"].clone())
        .collect()
}

/// The products of `records`, each its [`PRODUCT_FIELDS`]; a listing
/// page's Record holds no product ID.
pub(crate) fn products(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|record| record["value"]["id"].is_string())
        .map(|record| {
            PRODUCT_FIELDS
                .iter()
                .map(|field| ((*field).to_owned(), record["value"][*field].clone()))
                .collect::<serde_json::Map<String, Value>>()
                .into()
        })
        .collect()
}

/// The numbers of `records`, in their order.
pub(crate) fn numbers(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .map(|record| record["number"].clone())
        .collect()
}

/// Covers: story:3-23, row:RT-10, row:RT-11, row:durability-resume
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-005", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-005's cli variant starts");
    run.env()
        .user_configuration("daemon.idle_timeout = 60000\n");
    let mut pids: Vec<Value> = Vec::new();
    let mut settings = PathBuf::new();
    let mut before: Vec<Value> = Vec::new();
    let mut resumed = Value::Null;
    let mut after: Vec<Value> = Vec::new();
    let mut committed: Vec<Value> = Vec::new();

    run.step("Start the Daemon", |run| {
        let started = json_of(run.engine(), run.env(), &["daemon", "start", "--json"]);
        pids.push(started["pid"].clone());
    });
    run.step(
        "Start a Crawl of the catalog on the Daemon, reading its Records as they arrive",
        |run| {
            settings = start_crawl(run);
            before = first_records(run, BEFORE_THE_KILL);
        },
    );
    run.step(
        "Kill the Daemon (`kill -9`) once the Crawl has committed some Records",
        |run| {
            let pid = daemon_pid(run.env(), run.engine());
            assert_eq!(pid, pids[0], "the Daemon the person started still runs");
            kill(&pid);
            wait_until_stopped(run.env());
        },
    );
    run.step(
        "Start the Daemon again and resume the Crawl by its Run ID",
        |run| {
            let started = json_of(run.engine(), run.env(), &["daemon", "start", "--json"]);
            pids.push(started["pid"].clone());
            resumed = resume_crawl(run, &settings);
        },
    );
    run.step("Read the Crawl's Records until it finishes", |run| {
        let last = before
            .last()
            .map(|record| record["number"].to_string())
            .expect("Records were read before the kill");
        let read = command_line(run.env(), run.engine())
            .args(["run", "records", RUN, "--dataset", "pages", "--follow"])
            .args(["--after", &last, "--json"])
            .json_lines("RecordRead")
            .unwrap();
        after = read
            .documents
            .iter()
            .map(|read| read["record"].clone())
            .collect();
        let ended = command_line(run.env(), run.engine())
            .args(["run", "outcomes", RUN, "--follow", "--json"])
            .json_lines("CrawlRequestOutcome")
            .unwrap();
        assert!(!ended.documents.is_empty(), "{}", ended.output);
        committed = dataset(run);
    });

    run.check(
        "The kill lost none of the Records the Crawl committed before it: the Crawl resumed from its own state",
        |_| {
            assert_eq!(
                (
                    &committed[..before.len().min(committed.len())],
                    pids[0] != pids[1] && pids[1].is_u64(),
                    &resumed,
                ),
                (&before[..], true, &resumed_on(RUN)),
                "{pids:?}"
            );
        },
    );
    run.check(
        "No Record was delivered twice, across the kill and the resume",
        |_| {
            let delivered: Vec<Value> = before.iter().chain(&after).cloned().collect();
            let expected: Vec<Value> = (1..=committed.len()).map(|number| json!(number)).collect();
            let ids: BTreeSet<String> = products(&committed)
                .iter()
                .map(|product| product["id"].to_string())
                .collect();
            assert_eq!(
                (numbers(&delivered), &delivered, ids.len()),
                (expected, &committed, products(&committed).len()),
            );
        },
    );
    run.expect_task(
        "The oracle verifies the resumed Crawl's Records",
        &json!({ "status": "success", "records": products(&committed) }),
    );
    // The Daemon the person started again stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-005's document is written")
        .assert_proved();
}
