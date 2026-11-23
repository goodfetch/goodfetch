//! J-014, "Pause a Crawl, restart the Daemon and resume it", on the `cli`
//! surface (ADRs 0063 and 0066; tickets #220 and #2348): a person runs
//! Foxhound's shipped executable, starts a code-free crawl of the Proving
//! ground's catalog on the Daemon of the variant's home with `crawl
//! start`, pauses it with `run pause` once a Record committed, restarts
//! the Daemon with `daemon restart`, and resumes the crawl by its Run ID
//! with `run resume`, letting it finish with `run finish`.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the settings and fields files"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, ProvingGround, TestEnv, cli as command_line};
use serde_json::{Value, json};

use super::j002::{fields_read, products};
use crate::crawl_commands::short_idle;
use crate::fetch_tiers::settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The crawl's Run.
const RUN: &str = "paused";

/// The task whose Records the crawl returns.
const CATALOG: &str = "catalog/1";

/// A settings file for the crawl: the `http` tier, a short delay between
/// a domain's requests, so the pause comes partway through the catalog's
/// 110 pages, and the Proving ground's hosts mapped.
fn settings_file(env: &TestEnv, proving_ground: &ProvingGround) -> PathBuf {
    let path = env.folder().join("settings.json");
    let more = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "delay": 50 } },
    });
    std::fs::write(&path, settings(proving_ground, &more).to_string()).unwrap();
    path
}

/// The Records of the crawl's Dataset `pages`, as `run records --json`
/// prints them, through the number `through` when given.
fn records(run: &JourneyRun<impl Sized>, through: Option<u64>) -> Vec<Value> {
    let mut command = command_line(run.env(), run.engine()).args([
        "run",
        "records",
        RUN,
        "--dataset",
        "pages",
        "--json",
    ]);
    if let Some(through) = through {
        command = command.args(["--through", &through.to_string()]);
    }
    let read = command.json_array("RecordRead").unwrap();
    read.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|read| read["record"].clone())
        .collect()
}

/// Covers: story:3-35, row:RT-11, command:run.pause, command:run.resume, command:daemon.restart
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-014", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-014's cli variant starts");
    short_idle(run.env());
    let mut settings = PathBuf::new();
    let mut before: Vec<Value> = Vec::new();
    let mut paused = Value::Null;
    let mut after_restart = Value::Null;
    let mut resumed = Value::Null;
    let mut finished = Value::Null;
    let mut committed: Vec<Value> = Vec::new();

    run.step("Start a Crawl of the catalog on the Daemon", |run| {
        let proving_ground = run.proving_ground().expect("J-014 has a Proving ground");
        settings = settings_file(run.env(), proving_ground);
        let start = proving_ground.start_url(CATALOG).unwrap();
        let fields = run.env().folder().join("fields.json");
        std::fs::write(&fields, fields_read().to_string()).unwrap();
        let started = command_line(run.env(), run.engine())
            .args([
                "crawl", "start", &start, "--run-id", RUN, "--links", "scope",
            ])
            .args(["--fields", fields.to_str().unwrap()])
            .args(["--settings", settings.to_str().unwrap(), "--json"])
            .json("RunId")
            .unwrap();
        assert_eq!(started.documents[0], RUN, "{}", started.output);
    });
    run.step(
        "Pause the Crawl once it has committed some Records",
        |run| {
            before = records(run, Some(1));
            paused = command_line(run.env(), run.engine())
                .args(["run", "pause", RUN, "--json"])
                .json("RunReport")
                .unwrap()
                .documents[0]
                .clone();
        },
    );
    run.step("Restart the Daemon", |run| {
        let restarted = command_line(run.env(), run.engine())
            .args(["daemon", "restart"])
            .run()
            .unwrap();
        assert_eq!(restarted.exit, Some(0), "{restarted}");
        after_restart = command_line(run.env(), run.engine())
            .args(["run", "report", RUN, "--json"])
            .json("RunReport")
            .unwrap()
            .documents[0]
            .clone();
    });
    run.step("Resume the Crawl by its Run ID and let it finish", |run| {
        resumed = command_line(run.env(), run.engine())
            .args(["run", "resume", RUN])
            .args(["--settings", settings.to_str().unwrap(), "--json"])
            .json("RunStarted")
            .unwrap()
            .documents[0]
            .clone();
        finished = command_line(run.env(), run.engine())
            .args(["run", "finish", RUN, "--json"])
            .json("RunReport")
            .unwrap()
            .documents[0]
            .clone();
        committed = records(run, None);
    });

    run.check(
        "The pause stopped the Crawl as resumable, with its Checkpoint, and returned its Run report",
        |_| {
            let checkpoint = paused["checkpoint"]["location"].as_str().unwrap_or_default();
            assert_eq!(
                (
                    &paused["run_id"],
                    &paused["lifecycle"],
                    &paused["stop_reason"]["kind"],
                    Path::new(checkpoint).is_file(),
                ),
                (&json!(RUN), &json!("stopped"), &json!("pause"), true),
                "{paused}"
            );
        },
    );
    run.check(
        "After the restart the Crawl was still paused, and resuming it reached the same run rather than a new one",
        |_| {
            assert_eq!(
                (
                    &after_restart["lifecycle"],
                    &after_restart["stop_reason"]["kind"],
                    &resumed["run_id"],
                    &resumed["started_as"],
                    &finished["run_id"],
                    &finished["lifecycle"],
                ),
                (
                    &json!("stopped"),
                    &json!("pause"),
                    &json!(RUN),
                    &json!("resumed"),
                    &json!(RUN),
                    &json!("finished"),
                ),
                "{after_restart} {resumed} {finished}"
            );
        },
    );
    run.check(
        "No Record is in the Crawl's Dataset twice, across the pause and the resume",
        |_| {
            let urls: BTreeSet<&str> = committed
                .iter()
                .filter_map(|record| record["url"].as_str())
                .collect();
            let numbers: Vec<Value> = committed
                .iter()
                .map(|record| record["number"].clone())
                .collect();
            let expected: Vec<Value> = (1..=committed.len()).map(|number| json!(number)).collect();
            assert_eq!(
                (
                    !before.is_empty() && committed.starts_with(&before),
                    urls.len(),
                    numbers,
                ),
                (true, committed.len(), expected),
                "{before:?}"
            );
        },
    );
    run.expect_task(
        "The oracle verifies the resumed Crawl's Records",
        &json!({ "status": "success", "records": products(&committed) }),
    );
    // The Daemon the commands started stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-014's document is written")
        .assert_proved();
}
