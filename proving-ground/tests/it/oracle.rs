//! The oracle: it judges a submitted result for a task against the expected
//! result the seed gives, whatever status the tool reported, and names what
//! is missing, extra or wrong, the task it judged and its own time.
//!
//! The known-correct results come from reading the pages the way any tool
//! would: a crawl from the task's start page, an extract of its one page.

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 158;

/// The Record a product page shows.
fn read_product(proving_ground: &ProvingGround, url: &str) -> Value {
    let page = proving_ground.get(url).body;
    let field = |pattern: &str| {
        Regex::new(pattern)
            .expect("a valid pattern")
            .captures(&page)
            .expect(pattern)[1]
            .to_owned()
    };
    json!({
        "id": field(r#"<article class="product" data-id="([^"]+)""#),
        "name": field(r#"<h1 class="name">([^<]+)</h1>"#),
        "price": field(r#"data-price="([^"]+)""#),
    })
}

/// The Records of every product page reached from the listing pages, from
/// the start page through each Next link.
fn crawl(proving_ground: &ProvingGround, start_url: &str) -> Vec<Value> {
    let (origin, _) = start_url
        .split_once("/catalog/")
        .expect("a listing page's URL");
    let product = Regex::new(r#"href="(/products/[^"]+)""#).expect("a valid pattern");
    let next = Regex::new(r#"rel="next" href="([^"]+)""#).expect("a valid pattern");
    let mut records = Vec::new();
    let mut listing = Some(start_url.to_owned());
    while let Some(url) = listing {
        let page = proving_ground.get(&url).body;
        records.extend(
            product
                .captures_iter(&page)
                .map(|link| read_product(proving_ground, &format!("{origin}{}", &link[1]))),
        );
        listing = next
            .captures(&page)
            .map(|link| format!("{origin}{}", &link[1]));
    }
    records
}

/// A known-correct result for the task at `index` in the task list: its ID
/// and Records.
fn correct_result(proving_ground: &ProvingGround, index: usize) -> (String, Vec<Value>) {
    let task = proving_ground.tasks()["tasks"][index].clone();
    let start_url = task["start_url"].as_str().expect("a start URL");
    let records = match task["kind"].as_str().expect("a kind") {
        "crawl" => crawl(proving_ground, start_url),
        _ => vec![read_product(proving_ground, start_url)],
    };
    (task["id"].as_str().expect("an ID").to_owned(), records)
}

/// Covers: story:2-31, story:2-32
#[rstest]
#[case::crawl(0)]
#[case::extract(1)]
fn a_correct_result_is_verified(#[case] index: usize) {
    let proving_ground = ProvingGround::start(SEED);
    let (task, records) = correct_result(&proving_ground, index);

    let mut answer = proving_ground.judge(&submission(&task, "success", &records));

    let oracle_time = answer
        .as_object_mut()
        .and_then(|answer| answer.remove("oracle_time_microseconds"));
    assert!(oracle_time.is_some_and(|time| time.is_u64()), "{answer}");
    let quality = answer
        .as_object_mut()
        .and_then(|answer| answer.remove("quality"))
        .expect("the answer scores the result");
    assert_eq!(quality["correct"], quality["expected"], "{quality}");
    assert_eq!(
        answer,
        json!({
            "task": task,
            "outcome": "verified",
            "tool_status": "success",
            "missing": [],
            "extra": [],
            "wrong": [],
            "missing_effects": [],
            "extra_effects": [],
            "findings": [],
        })
    );
}

/// Covers: story:2-3, story:2-5
#[rstest]
#[case::needs_review("needs-review")]
#[case::failure("failure")]
fn a_correct_result_is_verified_whatever_the_tools_status(#[case] status: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let (task, records) = correct_result(&proving_ground, 0);

    let answer = proving_ground.judge(&submission(&task, status, &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(answer["tool_status"], status, "{answer}");
}

/// Covers: story:2-32
#[test]
fn a_result_missing_one_record_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 0);
    let dropped = records.pop().unwrap();

    let answer = proving_ground.judge(&submission(&task, "failure", &records));

    assert_eq!(answer["task"], task.as_str());
    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
    assert_eq!(answer["extra"], json!([]));
    assert_eq!(answer["wrong"], json!([]));
}

/// Covers: story:2-4, story:2-32
#[test]
fn a_success_missing_one_record_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 0);
    let dropped = records.remove(3);

    let answer = proving_ground.judge(&submission(&task, "success", &records));

    assert_eq!(answer["task"], task.as_str());
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

/// Covers: story:2-4, story:2-32
#[test]
fn a_success_with_a_wrong_price_is_a_false_success_naming_the_price() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 1);
    let shown = records[0]["price"].take();
    records[0]["price"] = json!("0.01");
    // Every price is at least 1.00, so its first digit is never the 0 of
    // 0.01.
    let first_digit = shown.as_str().unwrap().chars().next().unwrap();
    let difference = json!({
        "at": 0,
        "expected": format!("U+{:04X}", u32::from(first_digit)),
        "submitted": "U+0030",
    });

    let answer = proving_ground.judge(&submission(&task, "success", &records));

    assert_eq!(answer["task"], task.as_str());
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!([]));
    assert_eq!(
        answer["wrong"],
        json!([{
            "id": records[0]["id"],
            "field": "price",
            "expected": shown,
            "submitted": "0.01",
            "difference": difference,
        }])
    );
}

/// Covers: story:2-32
#[test]
fn a_failure_with_an_extra_record_is_failed_naming_the_record() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 1);
    let extra = json!({ "id": "P-9999", "name": "Unlisted", "price": "1.00" });
    records.push(extra.clone());

    let answer = proving_ground.judge(&submission(&task, "failure", &records));

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(answer["extra"], json!([extra]));
}

/// Covers: story:2-32
#[test]
fn a_record_submitted_twice_is_extra() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 1);
    records.push(records[0].clone());

    let answer = proving_ground.judge(&submission(&task, "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["extra"], json!([records[0]]));
}

#[test]
fn a_price_submitted_as_a_number_is_the_price_the_page_shows() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 1);
    let shown: f64 = records[0]["price"].as_str().unwrap().parse().unwrap();
    records[0]["price"] = json!(shown);

    let answer = proving_ground.judge(&submission(&task, "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn an_unknown_task_is_refused() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.submit(&submission("catalog/999", "success", &[]));

    assert_eq!(answer.status, 404);
    assert_eq!(answer.json()["error"], "unknown_task");
}

#[test]
fn a_malformed_submission_is_refused() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.submit(&json!({ "task": "catalog/1", "status": "done" }));

    assert_eq!(answer.status, 400);
    assert_eq!(answer.json()["error"], "malformed_submission");
}

/// The order task's ID and host, and the product its order form orders,
/// read from the form the way any tool would.
fn order_form(proving_ground: &ProvingGround) -> (String, String, String) {
    let task = proving_ground.tasks()["tasks"][2].clone();
    assert_eq!(task["kind"], "submit", "{task}");
    let start_url = task["start_url"].as_str().expect("a start URL");
    let form = proving_ground.get(start_url).body;
    assert!(
        form.contains(r#"<form method="post" action="/orders">"#),
        "{form}"
    );
    let product = Regex::new(r#"name="product" value="([^"]+)""#)
        .expect("a valid pattern")
        .captures(&form)
        .expect("the form names its product")[1]
        .to_owned();
    let host = start_url
        .strip_prefix("http://")
        .and_then(|rest| rest.split(':').next())
        .expect("a host")
        .to_owned();
    (
        task["id"].as_str().expect("an ID").to_owned(),
        host,
        product,
    )
}

/// Submits the order form `times` times.
fn place_order(proving_ground: &ProvingGround, host: &str, product: &str, times: usize) {
    for _ in 0..times {
        let answer = proving_ground.post_form(
            &format!("http://{host}/orders"),
            &[("product", product)],
            None,
        );
        assert_eq!(answer.status, 200, "{}", answer.body);
    }
}

#[test]
fn an_order_placed_once_is_verified_from_the_proving_grounds_own_record() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, host, product) = order_form(&proving_ground);
    place_order(&proving_ground, &host, &product, 1);

    let answer = proving_ground.judge(&submission(&task, "success", &[]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::claimed_a_success("success", "false_success")]
#[case::left_to_review("needs-review", "failed")]
fn an_order_placed_twice_is_judged_a_failure_from_the_proving_grounds_own_record(
    #[case] status: &str,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (task, host, product) = order_form(&proving_ground);
    place_order(&proving_ground, &host, &product, 2);

    let answer = proving_ground.judge(&submission(&task, status, &[]));

    assert_eq!(answer["outcome"], outcome, "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "order", "product": product }])
    );
    assert_eq!(answer["missing_effects"], json!([]));
}

#[test]
fn an_order_the_tool_reports_but_never_placed_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, _, product) = order_form(&proving_ground);

    let answer = proving_ground.judge(&submission(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["missing_effects"],
        json!([{ "kind": "order", "product": product }])
    );
}

#[test]
fn an_order_placed_on_another_tasks_host_is_not_the_order_tasks() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, _, product) = order_form(&proving_ground);
    place_order(&proving_ground, "1.catalog.test", &product, 1);

    let answer = proving_ground.judge(&submission(&task, "failure", &[]));

    assert_eq!(answer["outcome"], "failed", "{answer}");
}

#[test]
fn the_oracle_scores_each_field_against_the_ground_truth() {
    let proving_ground = ProvingGround::start(SEED);
    let (task, mut records) = correct_result(&proving_ground, 0);
    records.pop();
    records[0]["price"] = json!("0.01");

    let answer = proving_ground.judge(&submission(&task, "success", &records));

    assert_eq!(
        answer["quality"],
        json!({
            "fields": {
                "id": { "expected": 100, "correct": 99 },
                "name": { "expected": 100, "correct": 99 },
                "price": { "expected": 100, "correct": 98 },
            },
            "expected": 300,
            "correct": 296,
        })
    );
}
