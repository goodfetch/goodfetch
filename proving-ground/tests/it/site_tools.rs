//! The `site_tools` family: a page that registers WebMCP tools and removes
//! one of them once it has loaded, and the order its read-only tool answers.
//!
//! A plain HTTP client reads what the page holds. The tools it offers once
//! it shows `Ready` are this file's reading of the page's script, which a
//! browser with WebMCP runs.
//! Covers: story:2-33

use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{Answer, ProvingGround, submission};
use crate::reading::task;

const SEED: u64 = 1540;

const LIST_TASK: &str = "site_tools/1";
const ORDER_TASK: &str = "site_tools/2";

const PAGE: &str = "/tools";
const SCHEMA: &str = "/schemas/message.json";

/// The host name of the task `ordinal`.
fn host(ordinal: u32) -> String {
    format!("{ordinal}.site-tools.test")
}

/// The tools the page offers once it shows `Ready`: `track_parcel` is gone.
fn offered() -> Vec<Value> {
    vec![
        json!({ "id": "look_up_order", "read_only": true }),
        json!({ "id": "place_order", "read_only": false }),
        json!({ "id": "send_message", "read_only": false }),
    ]
}

/// The page of the task `ordinal`, fetched over plain HTTP.
fn page(proving_ground: &ProvingGround, ordinal: u32) -> Answer {
    let answer = proving_ground.get(&proving_ground.url(&host(ordinal), PAGE));
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer
}

/// The text of `body` between `before` and the next `after`.
fn between<'a>(body: &'a str, before: &str, after: &str) -> &'a str {
    body.split_once(before)
        .and_then(|(_, rest)| rest.split_once(after))
        .unwrap_or_else(|| panic!("{before} in {body}"))
        .0
}

/// The order the page holds, which `look_up_order` answers for its ID.
fn order(answer: &Answer) -> Value {
    let held = between(
        &answer.body,
        "<script type=\"application/json\" id=\"order\">",
        "</script>",
    );
    serde_json::from_str(held).expect("the order is JSON")
}

#[rstest]
#[case::the_list(LIST_TASK, 1)]
#[case::the_order(ORDER_TASK, 2)]
fn the_tasks_start_in_a_secure_context(#[case] id: &str, #[case] ordinal: u32) {
    let proving_ground = ProvingGround::start(SEED);

    let task = task(&proving_ground, id);

    assert_eq!(
        task["start_url"],
        format!("https://{}:{}{PAGE}", host(ordinal), proving_ground.port())
    );
}

#[test]
fn the_tools_offered_once_ready_are_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(LIST_TASK, "success", &offered()));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::the_removed_tool_too(json!({ "id": "track_parcel", "read_only": true }), None)]
#[case::a_tool_left_out(Value::Null, Some(2))]
#[case::a_tool_read_only_the_site_did_not_mark(json!({ "id": "place_order", "read_only": true }), Some(1))]
fn a_list_other_than_the_page_s_is_refuted(#[case] added: Value, #[case] dropped: Option<usize>) {
    let proving_ground = ProvingGround::start(SEED);
    let mut tools = offered();
    if let Some(at) = dropped {
        tools.remove(at);
    }
    if !added.is_null() {
        tools.push(added);
    }

    let answer = proving_ground.judge(&submission(LIST_TASK, "success", &tools));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn the_order_the_page_holds_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let answer = page(&proving_ground, 2);
    let order = order(&answer);
    let id = order["id"].as_str().expect("an order ID");
    assert_eq!(
        between(&answer.body, "<span id=\"order-id\">", "</span>"),
        id
    );

    let verdict = proving_ground.judge(&submission(ORDER_TASK, "success", &[order]));

    assert_eq!(verdict["outcome"], "verified", "{verdict}");
}

#[test]
fn another_seed_s_order_is_refuted() {
    let other = order(&page(&ProvingGround::start(SEED + 1), 2));
    let proving_ground = ProvingGround::start(SEED);
    assert_ne!(other, order(&page(&proving_ground, 2)));

    let verdict = proving_ground.judge(&submission(ORDER_TASK, "success", &[other]));

    assert_eq!(verdict["outcome"], "false_success", "{verdict}");
}

#[test]
fn a_seed_holds_one_order_on_every_page() {
    let proving_ground = ProvingGround::start(SEED);

    let first = order(&page(&proving_ground, 1));

    assert_eq!(first, order(&page(&proving_ground, 2)));
    assert_eq!(first, order(&page(&ProvingGround::start(SEED), 2)));
}

#[rstest]
#[case::the_list(LIST_TASK, 1)]
#[case::the_order(ORDER_TASK, 2)]
fn a_program_that_fetches_the_schema_s_reference_is_refuted(
    #[case] id: &str,
    #[case] ordinal: u32,
) {
    let proving_ground = ProvingGround::start(SEED);
    let records = match ordinal {
        1 => offered(),
        _ => vec![order(&page(&proving_ground, ordinal))],
    };
    let schema = proving_ground.get(&proving_ground.url(&host(ordinal), SCHEMA));
    assert_eq!(schema.status, 200, "{}", schema.body);
    assert_eq!(schema.json()["type"], "object");

    let verdict = proving_ground.judge(&submission(id, "success", &records));

    assert_ne!(verdict["outcome"], "verified", "{verdict}");
}

#[rstest]
#[case::another_path("/orders")]
#[case::another_schema("/schemas/order.json")]
fn every_other_path_is_not_found(#[case] path: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.get(&proving_ground.url(&host(1), path));

    assert_eq!(answer.status, 404, "{}", answer.body);
}
