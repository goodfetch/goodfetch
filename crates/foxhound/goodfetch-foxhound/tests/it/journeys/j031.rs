//! J-031, "Crawl with robots.txt turned off and find it in the Run report,
//! the Audit log and the Records' Provenance", on the `cli` surface (ADRs
//! 0063, 0075 and 0086): a person runs a crawl from a program, the test's
//! own client of Foxhound's Attached engine over stdio, with
//! `site_preferences.respect_robots_txt` off and
//! `site_preferences.mark_robots_txt_ignored` on, over a page the Proving
//! ground's robots.txt allows and one it disallows, reads the Run report,
//! exports the Audit log with Foxhound's shipped executable and reads the
//! crawl's Records.

use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::Attached;
use goodfetch_test_support::JourneyRun;
use serde_json::{Value, json};

use crate::attached_spawned;
use crate::politeness_records::{crawl, marks, records, settings};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The person's crawl.
const RUN_ID: &str = "robots-txt-off";

/// Covers: story:4-117, story:4-118, row:SC-01, row:compliance-site-owner-politeness
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-031", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-031's cli variant starts");
    let mut engine: Option<Attached> = None;
    let mut report = Value::Null;
    let mut exported: Vec<Value> = Vec::new();
    let mut read: Vec<Value> = Vec::new();

    run.step(
        "Run a crawl from a program over a page the site's robots.txt allows and one it disallows, with `site_preferences.respect_robots_txt` turned off and `site_preferences.mark_robots_txt_ignored` turned on for the run, each page's handler pushing a Record",
        |run| {
            let proving_ground = run.proving_ground().expect("J-031 has a Proving ground");
            let mut command = run.env().process_command(run.engine());
            command.arg("attached-engine");
            let mut program = attached_spawned(run.env(), None, &mut command);
            report = crawl(
                &mut program,
                proving_ground,
                RUN_ID,
                &["/catalog/1", "/private/catalog/1"],
                &settings(
                    proving_ground,
                    &json!({ "site_preferences": {
                        "respect_robots_txt": false,
                        "mark_robots_txt_ignored": true,
                    } }),
                ),
            )["result"]
                .clone();
            engine = Some(program);
        },
    );
    run.step("Read the run's report", |_| {
        assert!(!report.is_null(), "the run's report was read");
    });
    run.step("Export the Audit log", |run| {
        let output = run.command().args(["audit", "export"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        exported = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    });
    run.step("Read the crawl's Records", |_| {
        let mut program = engine.take().expect("the program is connected");
        read = records(&mut program, RUN_ID);
        assert!(program.finished());
    });

    run.check(
        "The Run report lists `site_preferences.respect_robots_txt` in `weakened`, set by the run",
        |_| {
            let listed: Vec<Value> = report["weakened"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|weakened| weakened["key"] == "site_preferences.respect_robots_txt")
                .map(|weakened| json!([weakened["value"], weakened["layer"]]))
                .collect();
            assert_eq!(listed, vec![json!([false, "run"])], "{report}");
        },
    );
    run.check(
        "The Audit log holds one `politeness_weakened` entry for the run, naming `site_preferences.respect_robots_txt`, its value `false`, its built-in default `true` and the run's layer",
        |_| {
            let entries: Vec<Value> = exported
                .iter()
                .filter(|entry| entry["kind"] == "politeness_weakened")
                .map(|entry| json!([entry["run_id"], entry["layer"], entry["details"]]))
                .collect();
            assert_eq!(
                entries,
                vec![json!([
                    RUN_ID,
                    "run",
                    {
                        "setting": "site_preferences.respect_robots_txt",
                        "value": false,
                        "default": true,
                    },
                ])]
            );
        },
    );
    run.check(
        "The Record of the disallowed page carries `robots_txt: \"ignored\"` in its Provenance, and the Record of the allowed page carries no mark",
        |_| {
            assert_eq!(
                marks(&read),
                vec![
                    ("/catalog/1".to_owned(), Value::Null),
                    ("/private/catalog/1".to_owned(), json!("ignored")),
                ]
            );
        },
    );
    run.finish()
        .expect("J-031's document is written")
        .assert_proved();
}
