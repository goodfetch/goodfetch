//! J-007, "Two Crawls of one site share Politeness, never more than its
//! in-flight cap together", on the `cli` surface (ADRs 0053, 0063, 0075 and
//! 0094; ticket #2360): a person sets the Proving ground's slow site's
//! Politeness to the 2 requests in flight it allows, starts two code-free
//! crawls of its list at once with Foxhound's shipped executable, each
//! `crawl start` reaching the one Daemon of the variant's home, follows
//! both to their end and reads each one's Records.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the settings and fields files"
)]

use std::io::Read as _;
use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, TestEnv, cli as command_line, record_command};
use serde_json::{Value, json};

use crate::crawl_commands::short_idle;
use crate::fetch_tiers::settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The two crawls, started at once.
const RUNS: [&str; 2] = ["first", "second"];

/// The fields each item's Record holds, read from its item page.
const ITEM_FIELDS: [&str; 2] = ["id", "name"];

/// The named fields the crawl reads from each page, as `--fields` takes
/// them: each item's ID from its article's attribute, and its name.
fn fields_read() -> Value {
    json!({
        "id": {
            "locator": { "kind": "css", "selector": "article.item" },
            "attribute": "data-id",
        },
        "name": { "locator": { "kind": "css", "selector": "article.item .name" } },
    })
}

/// The ID of the process the home's Daemon runs as, from `daemon status`.
fn daemon_pid(env: &TestEnv) -> Value {
    let status = command_line(env, FOXHOUND)
        .args(["daemon", "status", "--json"])
        .json("DaemonStatus")
        .unwrap();
    status.documents[0]["pid"].clone()
}

/// Starts both crawls of `start` and reads the Run ID each command prints.
/// The commands run at once and wait for the one Daemon, so a loaded host
/// delays both crawls, not one past the other's end.
fn start_both<C>(run: &JourneyRun<C>, start: &str, fields: &Path, settings: &Path) {
    let (fields, settings) = (fields.to_str().unwrap(), settings.to_str().unwrap());
    let arguments = RUNS.map(|run_id| {
        let mut arguments = vec!["crawl", "start", start, "--run-id", run_id];
        arguments.extend(["--format", "markdown", "--links", "scope", "--json"]);
        arguments.extend(["--fields", fields, "--settings", settings]);
        arguments
    });
    let started = arguments.each_ref().map(|arguments| {
        let mut command = run.env().process_command(run.engine());
        run.env().spawn(command.args(arguments)).unwrap()
    });
    for ((run_id, arguments), started) in RUNS.into_iter().zip(&arguments).zip(&started) {
        let status = started.close_stdin_and_wait().unwrap();
        record_command(run.env(), arguments, status.code()).unwrap();
        let mut answer = String::new();
        let mut stdout = started.child().stdout.take().unwrap();
        stdout.read_to_string(&mut answer).unwrap();
        let answer: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!((status.success(), answer), (true, json!(run_id)));
    }
}

/// The Records of `run_id`'s item pages, each its [`ITEM_FIELDS`], as `run
/// records` reads them; the list page's Record holds no item ID.
fn item_records<C>(run: &JourneyRun<C>, run_id: &str) -> Vec<Value> {
    let read = command_line(run.env(), run.engine())
        .args(["run", "records", run_id, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    read.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .filter(|read| read["record"]["value"]["id"].is_string())
        .map(|read| {
            ITEM_FIELDS
                .iter()
                .map(|field| ((*field).to_owned(), read["record"]["value"][*field].clone()))
                .collect::<serde_json::Map<String, Value>>()
                .into()
        })
        .collect()
}

/// The times the outcomes of a crawl ended, the first and the last.
fn span(outcomes: &[Value]) -> (String, String) {
    let mut ended: Vec<String> = outcomes
        .iter()
        .map(|outcome| outcome["ended_at"].as_str().unwrap().to_owned())
        .collect();
    ended.sort();
    (ended[0].clone(), ended[ended.len() - 1].clone())
}

/// Covers: story:3-24, row:CO-09, row:compliance-site-owner-politeness
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-007", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-007's cli variant starts");
    let mut settings_path = PathBuf::new();
    let mut pids: Vec<Value> = Vec::new();
    let mut outcomes: Vec<Vec<Value>> = Vec::new();
    let mut items: Vec<Vec<Value>> = Vec::new();
    short_idle(run.env());

    run.step(
        "Set the site's Politeness to at most 2 requests in flight",
        |run| {
            let proving_ground = run.proving_ground().expect("J-007 has a Proving ground");
            let politeness = json!({
                "fetch": { "tier": "http" },
                "politeness": { "domain": { "concurrency": 2, "delay": 0 } },
            });
            settings_path = run.env().folder().join("settings.json");
            std::fs::write(
                &settings_path,
                settings(proving_ground, &politeness).to_string(),
            )
            .unwrap();
        },
    );
    run.step(
        "Start two Crawls of the site's list at once, on one Daemon",
        |run| {
            let proving_ground = run.proving_ground().expect("J-007 has a Proving ground");
            let start = proving_ground.start_url("limits/3").unwrap();
            let fields = run.env().folder().join("fields.json");
            std::fs::write(&fields, fields_read().to_string()).unwrap();
            start_both(run, &start, &fields, &settings_path);
            pids.push(daemon_pid(run.env()));
        },
    );
    run.step("Wait for both Crawls to finish", |run| {
        for run_id in RUNS {
            let ended = command_line(run.env(), run.engine())
                .args(["run", "outcomes", run_id, "--follow", "--json"])
                .json_lines("CrawlRequestOutcome")
                .unwrap();
            outcomes.push(ended.documents);
            items.push(item_records(run, run_id));
        }
        pids.push(daemon_pid(run.env()));
    });

    for (check, records) in [
        "The oracle verifies the first Crawl's Records",
        "The oracle verifies the second Crawl's Records",
    ]
    .into_iter()
    .zip(&items)
    {
        run.expect_task(check, &json!({ "status": "success", "records": records }));
    }
    run.check(
        "The Proving ground saw no request beyond the site's 2 in flight, from either Crawl",
        |run| {
            let proving_ground = run.proving_ground().expect("J-007 has a Proving ground");
            let observations = proving_ground.observations().unwrap();
            let attempts: Vec<Value> = outcomes
                .iter()
                .flatten()
                .filter(|outcome| outcome["kind"] != "succeeded" || outcome["attempts"] != 1)
                .cloned()
                .collect();
            assert_eq!(
                (&observations["findings"], attempts),
                (&json!([]), Vec::new())
            );
        },
    );
    run.check("Both Crawls ran at once in the one Daemon process", |_| {
        let ((first_began, first_ended), (second_began, second_ended)) =
            (span(&outcomes[0]), span(&outcomes[1]));
        assert_eq!(
            (
                pids[0].is_u64() && pids[0] == pids[1],
                first_began < second_ended && second_began < first_ended,
            ),
            (true, true),
            "{pids:?} {first_began}..{first_ended} {second_began}..{second_ended}"
        );
    });
    // The Daemon the commands started stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-007's document is written")
        .assert_proved();
}
