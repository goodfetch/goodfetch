//! J-042, "Erase a Dataset and see it gone", on the `cli` surface (ADR
//! 0031; spec #1937, issue 1220): a person crawls a Markdown page of the
//! Proving ground with Foxhound's shipped executable, as a code-free run
//! on the Daemon, reads its Dataset, erases it with `dataset erase` in a
//! pseudo-terminal, answering yes once warned, then reads it again.

use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, cli as command_line, pty};
use serde_json::Value;

use crate::journeys::j001::json_of;
use crate::journeys::j005::crawl_settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The task whose index links Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// The run the person crawls.
const RUN_ID: &str = "erased";

/// A word of the index's page.
const WORD: &str = "fenced";

/// The question `dataset erase` asks.
const QUESTION: &str = "Erase the Dataset `pages` of the run `erased`? [y/N]";

/// How many Records each read of the Dataset finds: a page of it, `run
/// records` and a search for [`WORD`].
fn read(run: &JourneyRun<impl Sized>) -> [usize; 3] {
    let count = |value: &Value| value.as_array().map_or(usize::MAX, Vec::len);
    let page = command_line(run.env(), run.engine())
        .args(["dataset", "page", "--run-id", RUN_ID, "--dataset", "pages"])
        .arg("--json")
        .json("DatasetPage")
        .unwrap();
    let records = command_line(run.env(), run.engine())
        .args(["run", "records", RUN_ID, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    let searched = command_line(run.env(), run.engine())
        .args([
            "dataset",
            "search",
            "--run-id",
            RUN_ID,
            "--dataset",
            "pages",
        ])
        .args(["--text", WORD, "--json"])
        .json("DatasetSearchPage")
        .unwrap();
    [
        count(&page.documents[0]["records"]),
        count(&records.documents[0]),
        count(&searched.documents[0]["records"]),
    ]
}

/// Covers: story:12-214, command:dataset.erase
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-042", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-042's cli variant starts");
    run.env()
        .user_configuration("daemon.idle_timeout = 60000\n");
    let mut before = [0; 3];
    let mut after = [usize::MAX; 3];
    let mut prompted = String::new();

    run.step(
        "Crawl a page of a site from the command line, as a run on the Daemon",
        |run| {
            json_of(run.engine(), run.env(), &["daemon", "start", "--json"]);
            let settings = crawl_settings(run);
            let index = run
                .proving_ground()
                .expect("J-042 has a Proving ground")
                .start_url(MARKDOWN_TASK)
                .unwrap();
            let started = command_line(run.env(), run.engine())
                .args(["crawl", "start", &index])
                .args([
                    "--run-id", RUN_ID, "--format", "markdown", "--links", "none",
                ])
                .args(["--settings", settings.to_str().unwrap(), "--json"])
                .json("RunId")
                .unwrap();
            assert_eq!(started.documents[0], RUN_ID, "{}", started.output);
            // The crawl has ended once its last ending is read.
            let followed = command_line(run.env(), run.engine())
                .args(["run", "outcomes", RUN_ID, "--follow", "--json"])
                .run()
                .unwrap();
            assert_eq!(followed.exit, Some(0), "{followed}");
        },
    );
    run.step("Read the run's Dataset (`dataset page`)", |run| {
        before = read(run);
    });
    run.step(
        "Erase the Dataset at the terminal, answering yes once warned (`dataset erase`)",
        |run| {
            let mut erase = pty(run.env(), run.engine())
                .args(["dataset", "erase", "--run-id", RUN_ID, "--dataset", "pages"])
                .spawn()
                .unwrap();
            erase.expect(QUESTION).unwrap();
            erase.send_line("y").unwrap();
            let erased = erase.finish().unwrap();
            assert_eq!(erased.exit, Some(0), "{}", erased.stripped());
            prompted = erased.stripped();
        },
    );
    run.step(
        "Read the Dataset again, and search it for a word of the page",
        |run| {
            after = read(run);
        },
    );

    run.check(
        "The command warned that the erase cannot be undone and asked once",
        |_| {
            assert_eq!(
                (
                    prompted.contains("cannot be undone"),
                    prompted.matches("[y/N]").count()
                ),
                (true, 1),
                "{prompted}"
            );
        },
    );
    run.check(
        "The Dataset held the page's Record before the erase, and every read after it finds none",
        |_| {
            assert_eq!((before, after), ([1, 1, 1], [0, 0, 0]));
        },
    );
    // The Daemon the person started stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-042's document is written")
        .assert_proved();
}
