//! The `print` family (#1536): a report whose `@media print` rules show and
//! hide other entries than its screen rules, saved as print, as screen and
//! as one MHTML file. The entries the right medium shows are verified; the
//! screen's entries returned for the print task are a false success.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 1536;

/// The `id` of each Record in `records`.
fn ids(records: &Value) -> Vec<&Value> {
    records
        .as_array()
        .expect("an array of Records")
        .iter()
        .map(|record| &record["id"])
        .collect()
}

#[test]
fn the_report_holds_every_entry_and_its_stylesheet_hides_them_by_medium() {
    let proving_ground = ProvingGround::start(SEED);

    let report = proving_ground.get(&browser::start_url(&proving_ground, "print/1"));
    let stylesheet = proving_ground.get(&proving_ground.url("1.print.test", "/report.css"));

    assert_eq!(
        report.body.matches("<section class=\"entry").count(),
        6,
        "{}",
        report.body
    );
    assert_eq!(report.body.matches("screen-only").count(), 2);
    assert_eq!(report.body.matches("print-only").count(), 2);
    assert!(
        stylesheet
            .body
            .contains("@media print {\n  .screen-only { display: none; }\n}"),
        "{}",
        stylesheet.body
    );
    assert!(
        stylesheet
            .body
            .contains("@media screen {\n  .print-only { display: none; }\n}"),
        "{}",
        stylesheet.body
    );
}

#[test]
fn the_entries_the_page_prints_are_verified_for_the_print_task() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(
        &proving_ground,
        "print/1",
        "entries",
        json!({ "media": "print" }),
    );

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn the_entries_the_screen_shows_are_a_false_success_for_the_print_task() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(
        &proving_ground,
        "print/1",
        "entries",
        json!({ "media": "screen" }),
    );

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!(["P-3", "P-5"]), "{answer}");
    assert_eq!(
        ids(&answer["extra"]),
        [&json!("P-2"), &json!("P-6")],
        "{answer}"
    );
}

#[test]
fn the_entries_the_screen_shows_are_verified_for_the_screen_task() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(
        &proving_ground,
        "print/2",
        "entries",
        json!({ "media": "screen" }),
    );

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn the_parts_of_the_page_saved_as_one_mhtml_file_are_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "print/3", "mhtml", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}
