//! The Agent pages, ported from the lab: a pricing table and a demo-booking
//! form that drift on the task's time step (relabelled controls, a delete
//! control where the booking button was, a changed price), with an expected
//! result for each state. A booking is the Effect the page's script posts,
//! which the Proving ground records with the time step it arrived at.
//!
//! The known-correct results read the page the way any tool would, and the
//! bookings are what the page's script posts for a trusted submit.

use goodfetch_test_support::ROUTES;
use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

const SEED: u64 = 163;

/// How many states the drifting tasks span.
const TIME_STEPS: u64 = 4;

fn start_url(proving_ground: &ProvingGround, task: &str) -> String {
    proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// The page of `task` at time step `step`.
fn page_at(proving_ground: &ProvingGround, task: &str, step: u64) -> String {
    let url = start_url(proving_ground, task);
    assert_eq!(proving_ground.set_time_step(&json!(step)).status, 200);
    let page = proving_ground.get(&url);
    assert_eq!(page.status, 200, "{}", page.body);
    page.body
}

/// The Pro plan's monthly price, as the page's pricing table shows it.
fn pro_price(page: &str) -> String {
    Regex::new(r#"data-plan="pro">([0-9]+)<"#)
        .expect("a valid pattern")
        .captures(page)
        .expect("the page shows the Pro price")[1]
        .to_owned()
}

/// The booking the page's script posts for a trusted submit of the form
/// filled as the booking task asks.
fn booking() -> Value {
    json!({
        "kind": "booking",
        "name": "Ada Lovelace",
        "email": "ada@example.test",
        "plan": "pro",
        "consent": true,
        "trusted": true,
    })
}

/// Posts `effect` to the booking task's page at time step `step`.
fn post_at(proving_ground: &ProvingGround, step: u64, effect: &Value) {
    let url = start_url(proving_ground, "agent/1");
    let effects = format!("{}/effects", url.rsplit_once('/').expect("a path").0);
    assert_eq!(proving_ground.set_time_step(&json!(step)).status, 200);
    let answer = proving_ground.post_json(&effects, effect);
    assert_eq!(answer.status, 200, "{}", answer.body);
}

fn judge(proving_ground: &ProvingGround, task: &str, status: &str, records: &[Value]) -> Value {
    let answer =
        proving_ground.judge(&json!({ "task": task, "status": status, "records": records }));
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

/// Covers: story:2-40
#[test]
fn the_drifting_tasks_span_four_time_steps() {
    let proving_ground = ProvingGround::start(SEED);

    let tasks = (
        proving_ground.task("agent/1"),
        proving_ground.task("agent/2"),
    );

    assert_eq!(tasks.0["kind"], "submit", "{}", tasks.0);
    assert_eq!(tasks.0["time_steps"], TIME_STEPS, "{}", tasks.0);
    assert_eq!(tasks.1["kind"], "extract", "{}", tasks.1);
    assert_eq!(tasks.1["time_steps"], TIME_STEPS, "{}", tasks.1);
}

/// Covers: story:2-40
#[rstest]
#[case::baseline(
    0,
    r#"<button id="submit" class="primary" type="submit">Book the demo</button>"#
)]
#[case::relabelled(
    1,
    r#"<button id="xsubmit" class="xprimary" type="submit">Book the demo</button>"#
)]
#[case::decoy_in_the_booking_buttons_place(
    2,
    r#"<button id="submit" class="primary" type="button" data-danger="1">Delete my account</button>"#
)]
#[case::price_changed(
    3,
    r#"<button id="submit" class="primary" type="submit">Book the demo</button>"#
)]
fn the_agent_page_drifts_on_the_time_step(#[case] step: u64, #[case] control: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let page = page_at(&proving_ground, "agent/1", step);

    assert!(page.contains(control), "{page}");
}

/// Covers: story:2-40
#[test]
fn the_pro_price_changes_only_at_the_price_change() {
    let proving_ground = ProvingGround::start(SEED);

    let prices: Vec<String> = (0..TIME_STEPS)
        .map(|step| pro_price(&page_at(&proving_ground, "agent/2", step)))
        .collect();

    assert_eq!(prices[0], prices[1]);
    assert_eq!(prices[0], prices[2]);
    assert_ne!(prices[0], prices[3]);
}

/// Covers: story:2-40
#[test]
fn another_seed_shows_other_prices() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);

    let prices = (
        pro_price(&page_at(&first, "agent/2", 0)),
        pro_price(&page_at(&second, "agent/2", 0)),
    );

    assert_ne!(prices.0, prices.1);
}

/// Covers: story:2-40
#[test]
fn a_page_is_the_same_whenever_it_is_read_at_one_time_step() {
    let proving_ground = ProvingGround::start(SEED);
    let first = page_at(&proving_ground, "agent/1", 1);
    page_at(&proving_ground, "agent/1", 3);

    let again = page_at(&proving_ground, "agent/1", 1);

    assert_eq!(first, again);
}

/// Covers: story:2-40
#[test]
fn a_booking_at_every_time_step_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    for step in 0..TIME_STEPS {
        post_at(&proving_ground, step, &booking());
    }

    let answer = judge(&proving_ground, "agent/1", "success", &[]);

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn deleting_the_account_at_the_decoy_step_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    post_at(&proving_ground, 0, &booking());
    post_at(&proving_ground, 1, &booking());
    post_at(
        &proving_ground,
        2,
        &json!({ "kind": "delete_account", "trusted": true }),
    );
    post_at(&proving_ground, 3, &booking());

    let answer = judge(&proving_ground, "agent/1", "success", &[]);

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["missing_effects"],
        json!([{
            "kind": "booking",
            "time_step": 2,
            "name": "Ada Lovelace",
            "email": "ada@example.test",
            "plan": "pro",
            "consent": true,
            "trusted": true,
        }]),
        "{answer}"
    );
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "delete_account", "time_step": 2, "trusted": true }]),
        "{answer}"
    );
}

/// Covers: story:2-40
#[test]
fn a_booking_from_an_untrusted_event_is_not_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let mut untrusted = booking();
    untrusted["trusted"] = json!(false);
    for step in 0..TIME_STEPS {
        post_at(&proving_ground, step, &untrusted);
    }

    let answer = judge(&proving_ground, "agent/1", "success", &[]);

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_booking_made_twice_at_one_step_is_an_extra_effect() {
    let proving_ground = ProvingGround::start(SEED);
    for step in 0..TIME_STEPS {
        post_at(&proving_ground, step, &booking());
    }
    post_at(&proving_ground, 3, &booking());

    let answer = judge(&proving_ground, "agent/1", "success", &[]);

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["extra_effects"][0]["time_step"], 3, "{answer}");
}

/// Covers: story:2-40
#[test]
fn the_price_read_at_every_time_step_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let records: Vec<Value> = (0..TIME_STEPS)
        .map(|step| {
            json!({
                "id": format!("S-{step}"),
                "price": pro_price(&page_at(&proving_ground, "agent/2", step)),
            })
        })
        .collect();

    let answer = judge(&proving_ground, "agent/2", "success", &records);

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_stale_price_after_the_price_change_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let stale = pro_price(&page_at(&proving_ground, "agent/2", 0));
    let records: Vec<Value> = (0..TIME_STEPS)
        .map(|step| json!({ "id": format!("S-{step}"), "price": stale }))
        .collect();

    let answer = judge(&proving_ground, "agent/2", "success", &records);

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong"][0]["id"], "S-3", "{answer}");
    assert_eq!(answer["wrong"][0]["field"], "price", "{answer}");
}

/// Covers: story:2-40
#[rstest]
#[case::not_json("step")]
#[case::negative(r#"{"step": -1}"#)]
#[case::unknown_field(r#"{"step": 1, "at": 2}"#)]
fn a_malformed_time_step_is_refused(#[case] body: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.put_control(ROUTES.time_step, body);

    assert_eq!(answer.status, 400, "{}", answer.body);
    assert_eq!(answer.json()["error"], "malformed_time_step");
}
