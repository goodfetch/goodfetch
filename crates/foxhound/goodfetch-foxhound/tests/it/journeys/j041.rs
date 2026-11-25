//! J-041, "Find which runs returned a Record with search across runs, then
//! query them with the SQL tab", on the `cli` surface (ADRs 0031, 0063 and
//! 0075; spec #1937, issue 1217): a person crawls three Markdown pages of
//! the Proving ground with Foxhound's shipped executable, each a code-free
//! run of its own on the Daemon, searches the three runs for a word with
//! `record search`, then reads the runs it found with `dataset query`.

use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, cli as command_line};
use serde_json::{Value, json};

use crate::journeys::j001::json_of;
use crate::journeys::j005::crawl_settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The task whose index links Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// The word the person searches for: in the index, which links the page of
/// fenced code, and in that page, and in neither the product's page.
const WORD: &str = "fenced";

/// Each run and the path of the page it crawls, below the index.
const RUNS: [(&str, &str); 3] = [
    ("monday", ""),
    ("tuesday", "/code/fenced-language"),
    ("wednesday", "/mixed/product"),
];

/// Each Record's run and number, in that order.
fn found_in(records: &Value) -> Vec<(String, u64)> {
    let mut found: Vec<(String, u64)> = records
        .as_array()
        .unwrap_or_else(|| panic!("{records}"))
        .iter()
        .map(|record| {
            (
                record[0].as_str().unwrap_or_default().to_owned(),
                record[1].as_u64().unwrap_or_default(),
            )
        })
        .collect();
    found.sort();
    found
}

/// Covers: story:12-209, story:12-210, command:record.search, command:dataset.query
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-041", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-041's cli variant starts");
    run.env()
        .user_configuration("daemon.idle_timeout = 60000\n");
    let mut found = Value::Null;
    let mut queried = Value::Null;

    run.step(
        "Crawl three pages of a site from the command line, each as a run of its own on the Daemon",
        |run| {
            json_of(run.engine(), run.env(), &["daemon", "start", "--json"]);
            let settings = crawl_settings(run);
            let index = run
                .proving_ground()
                .expect("J-041 has a Proving ground")
                .start_url(MARKDOWN_TASK)
                .unwrap();
            for (run_id, path) in RUNS {
                let started = command_line(run.env(), run.engine())
                    .args(["crawl", "start", &format!("{index}{path}")])
                    .args([
                        "--run-id", run_id, "--format", "markdown", "--links", "none",
                    ])
                    .args(["--settings", settings.to_str().unwrap(), "--json"])
                    .json("RunId")
                    .unwrap();
                assert_eq!(started.documents[0], run_id, "{}", started.output);
                // The crawl has ended once its last ending is read.
                let followed = command_line(run.env(), run.engine())
                    .args(["run", "outcomes", run_id, "--follow", "--json"])
                    .run()
                    .unwrap();
                assert_eq!(followed.exit, Some(0), "{followed}");
            }
        },
    );
    run.step(
        "Search the three runs for a word with search across runs (`record search`)",
        |run| {
            let searched = command_line(run.env(), run.engine())
                .args(["record", "search", "--text", WORD])
                .args(RUNS.iter().flat_map(|(run_id, _)| ["--runs", *run_id]))
                .arg("--json")
                .json("RecordSearchResult")
                .unwrap();
            assert_eq!(searched.output.exit, Some(0), "{}", searched.output);
            found = Value::Array(
                searched.documents[0]["records"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{}", searched.output))
                    .iter()
                    .map(|record| json!([record["run_id"], record["number"]]))
                    .collect(),
            );
        },
    );
    run.step(
        "Query the runs the search found with the SQL tab (`dataset query`)",
        |run| {
            let runs: Vec<(String, u64)> = found_in(&found);
            let sql = runs
                .iter()
                .map(|(run_id, _)| {
                    format!("SELECT '{run_id}' AS run, number FROM records('{run_id}')")
                })
                .collect::<Vec<_>>()
                .join(" UNION ALL ");
            let queried_runs = command_line(run.env(), run.engine())
                .args(["dataset", "query", "--sql", &sql])
                .args(
                    runs.iter()
                        .flat_map(|(run_id, _)| ["--runs", run_id.as_str()]),
                )
                .arg("--json")
                .json("DatasetQueryResult")
                .unwrap();
            assert_eq!(queried_runs.output.exit, Some(0), "{}", queried_runs.output);
            queried = queried_runs.documents[0]["rows"].clone();
        },
    );

    run.check(
        "The search found the Record of each run whose page holds the word, and none of the run whose page does not",
        |_| {
            assert_eq!(
                found_in(&found),
                vec![("monday".to_owned(), 1), ("tuesday".to_owned(), 1)],
                "{found}"
            );
        },
    );
    run.check(
        "The query over the runs the search found read the same Records",
        |_| {
            assert_eq!(found_in(&queried), found_in(&found), "{queried}");
        },
    );
    // The Daemon the person started stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-041's document is written")
        .assert_proved();
}
