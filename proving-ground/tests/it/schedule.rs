//! The `schedule` family: a board whose content changes on a known schedule
//! of time steps, for change tracking. The time step is the Proving ground's
//! own, set by whoever runs the task (`PUT /time-step`), never the wall
//! clock: the board at a step is the same whenever it is read.
//!
//! The known-correct results come from reading the board at each step of the
//! task over plain HTTP and keeping each revision it shows once.

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 166;
const HOST: &str = "schedule.test";

fn board(proving_ground: &ProvingGround, step: u64) -> String {
    let set = proving_ground.set_time_step(&json!(step));
    assert_eq!(set.status, 200, "{}", set.body);
    let page = proving_ground.get(&proving_ground.url(HOST, "/board"));
    assert_eq!(page.status, 200, "{}", page.body);
    page.body
}

fn capture(text: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .unwrap_or_else(|| panic!("{pattern} in {text}"))[1]
        .to_owned()
}

fn revision(page: &str) -> String {
    capture(page, r#"<article class="notice" data-revision="([^"]+)""#)
}

/// A known-correct result for the watch task: one Record per revision the
/// board shows over the task's time steps.
fn watch(proving_ground: &ProvingGround) -> Vec<Value> {
    let task = proving_ground.task("schedule/1");
    let steps = task["time_steps"].as_u64().expect("a count of time steps");
    let mut records: Vec<Value> = Vec::new();
    for step in 0..steps {
        let page = board(proving_ground, step);
        let record = json!({
            "id": revision(&page),
            "headline": capture(&page, r#"<h1 class="headline">([^<]+)</h1>"#),
            "price": capture(&page, r#"data-price="([^"]+)""#),
        });
        if records.last() != Some(&record) {
            records.push(record);
        }
    }
    records
}

/// Covers: story:2-33
#[test]
fn the_time_step_starts_at_zero() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&proving_ground.url(HOST, "/board"));

    assert_eq!(revision(&page.body), "R-0");
}

/// Covers: story:2-33
#[rstest]
#[case::changed_at_one(1, "R-1")]
#[case::unchanged_at_two(2, "R-1")]
#[case::changed_at_three(3, "R-2")]
#[case::changed_at_four(4, "R-3")]
#[case::unchanged_at_five(5, "R-3")]
fn the_board_shows_the_revision_its_schedule_gives_the_time_step(
    #[case] step: u64,
    #[case] expected: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let page = board(&proving_ground, step);

    assert_eq!(revision(&page), expected);
}

/// Covers: story:2-30, story:2-33
#[test]
fn the_board_is_the_same_whenever_it_is_read_at_one_time_step() {
    let first = ProvingGround::start(SEED);
    let earlier = board(&first, 3);
    board(&first, 1);
    let second = ProvingGround::start(SEED);

    let pages = (board(&first, 3), board(&second, 3));

    assert_eq!(pages.0, earlier);
    assert_eq!(pages.1, earlier);
}

/// Covers: story:2-30, story:2-33
#[test]
fn another_seed_shows_another_board() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);

    let pages = (board(&first, 0), board(&second, 0));

    assert_ne!(pages.0, pages.1);
}

#[rstest]
#[case::negative(json!(-1))]
#[case::fractional(json!(1.5))]
#[case::text(json!("1"))]
fn a_time_step_that_is_not_a_whole_number_is_refused(#[case] step: Value) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.set_time_step(&step);

    assert_eq!(answer.status, 400, "{}", answer.body);
    assert_eq!(answer.json()["error"], "malformed_time_step");
}

/// Covers: story:2-31, story:2-33
#[test]
fn a_correct_watch_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let records = watch(&proving_ground);

    let answer = proving_ground.judge(&submission("schedule/1", "success", &records));

    assert_eq!(answer["task"], "schedule/1");
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-32, story:2-33
#[test]
fn a_watch_missing_one_revision_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = watch(&proving_ground);
    records.remove(2);

    let answer = proving_ground.judge(&submission("schedule/1", "failure", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!(["R-2"]));
}

/// Covers: story:2-4, story:2-33
#[test]
fn a_watch_that_reports_each_visit_as_a_change_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = watch(&proving_ground);
    let repeated = records[1].clone();
    records.insert(2, repeated.clone());

    let answer = proving_ground.judge(&submission("schedule/1", "success", &records));

    assert_eq!(answer["task"], "schedule/1");
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["extra"], json!([repeated]));
}

/// Covers: story:2-4, story:2-33
#[test]
fn a_watch_that_stops_at_its_first_read_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = watch(&proving_ground);
    records.truncate(1);

    let answer = proving_ground.judge(&submission("schedule/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!(["R-1", "R-2", "R-3"]));
}
