//! J-014, "Pause a Crawl, restart the Daemon and resume it", on the `cli`
//! surface (ADRs 0063, 0075 and 0094): a person starts a code-free crawl of
//! the Proving ground's catalog on the Daemon with Foxhound's shipped
//! executable, pauses it once some Records are committed, restarts the
//! Daemon, checks the crawl is still paused and resumes it by its Run ID,
//! then reads its Records once it finishes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, cli as command_line};
use serde_json::{Value, json};

use crate::journeys::j001::json_of;
use crate::journeys::j005::{
    RUN, dataset, first_records, numbers, products, resume_crawl, resumed_on, start_crawl,
};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// How many Records the person reads before pausing the crawl.
const BEFORE_THE_PAUSE: usize = 3;

/// The stop reason of a Run its pause stopped.
fn paused_for() -> Value {
    json!({ "kind": "pause" })
}

/// Covers: story:3-35, row:RT-11
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant reads as its file: each step, then each check, in order"
)]
fn cli() {
    let mut run = JourneyRun::start("J-014", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-014's cli variant starts");
    run.env()
        .user_configuration("daemon.idle_timeout = 60000\n");
    let mut settings = PathBuf::new();
    let mut before: Vec<Value> = Vec::new();
    let mut pause_exit = None;
    let mut paused = Value::Null;
    let mut restarted = Value::Null;
    let mut after_the_restart = Value::Null;
    let mut resumed = Value::Null;
    let mut ended = Value::Null;
    let mut committed: Vec<Value> = Vec::new();

    run.step("Start a Crawl of the catalog on the Daemon", |run| {
        json_of(run.engine(), run.env(), &["daemon", "start", "--json"]);
        settings = start_crawl(run);
    });
    run.step(
        "Pause the Crawl once it has committed some Records",
        |run| {
            before = first_records(run, BEFORE_THE_PAUSE);
            let output = command_line(run.env(), run.engine())
                .args(["run", "pause", RUN, "--json"])
                .run()
                .unwrap();
            pause_exit = output.exit;
            paused = serde_json::from_str(&output.stdout)
                .unwrap_or_else(|error| panic!("{error}: {}{}", output.stdout, output.stderr));
        },
    );
    run.step("Restart the Daemon", |run| {
        restarted = json_of(run.engine(), run.env(), &["daemon", "restart", "--json"]);
    });
    run.step("Resume the Crawl by its Run ID and let it finish", |run| {
        after_the_restart = json_of(run.engine(), run.env(), &["run", "progress", RUN, "--json"]);
        resumed = resume_crawl(run, &settings);
        ended = json_of(run.engine(), run.env(), &["run", "report", RUN, "--json"]);
        committed = dataset(run);
    });

    run.check(
        "The pause stopped the Crawl as resumable, with its Checkpoint, and returned its Run report",
        |_| {
            // `20`: the Run ended `stopped`, resumable by its Run ID.
            assert_eq!(
                (
                    pause_exit,
                    &paused["run_id"],
                    &paused["lifecycle"],
                    &paused["stop_reason"],
                    &paused["checkpoint"]["run_id"],
                ),
                (Some(20), &json!(RUN), &json!("stopped"), &paused_for(), &json!(RUN)),
                "{paused}"
            );
        },
    );
    run.check(
        "After the restart the Crawl was still paused, and resuming it reached the same run rather than a new one",
        |_| {
            assert!(restarted["pid"].is_u64(), "{restarted}");
            assert_eq!(
                (
                    &after_the_restart["lifecycle"],
                    &after_the_restart["stop_reason"],
                    &resumed,
                    &ended["run_id"],
                    &ended["lifecycle"],
                ),
                (
                    &json!("stopped"),
                    &paused_for(),
                    &resumed_on(RUN),
                    &json!(RUN),
                    &json!("finished"),
                ),
                "{after_the_restart} {resumed} {ended}"
            );
        },
    );
    run.check(
        "No Record is in the Crawl's Dataset twice, across the pause and the resume",
        |_| {
            let ids: BTreeSet<String> = products(&committed)
                .iter()
                .map(|product| product["id"].to_string())
                .collect();
            let expected: Vec<Value> = (1..=committed.len()).map(|number| json!(number)).collect();
            assert_eq!(
                (
                    numbers(&committed),
                    &committed[..BEFORE_THE_PAUSE.min(committed.len())],
                    ids.len(),
                ),
                (expected, &before[..], products(&committed).len()),
            );
        },
    );
    run.expect_task(
        "The oracle verifies the resumed Crawl's Records",
        &json!({ "status": "success", "records": products(&committed) }),
    );
    // The Daemon the person restarted stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-014's document is written")
        .assert_proved();
}
