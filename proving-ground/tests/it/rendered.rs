//! The `rendered` family: a feed that loads its items by AJAX as it is
//! scrolled, and item pages whose name sits in a shadow root, whose price is
//! loaded by AJAX and whose description is in a frame. None of it is in the
//! pages' own HTML; a browser reaches it by running their scripts.
//!
//! The known-correct results come from reading the pages over plain HTTP the
//! way their scripts do: the feed's pages from its `data-source`, each item's
//! data from its own, and its description from its frame.

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 166;
const HOST: &str = "rendered.test";

/// The first capture of `pattern` in `text`.
fn capture(text: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .unwrap_or_else(|| panic!("{pattern} in {text}"))[1]
        .to_owned()
}

/// The Record an item page shows once its scripts have run.
fn read_item(proving_ground: &ProvingGround, url: &str) -> Value {
    let page = proving_ground.get(url).body;
    let data = proving_ground
        .get(&proving_ground.url(
            HOST,
            &capture(&page, r#"<item-card [^>]*data-source="([^"]+)""#),
        ))
        .json();
    let frame = proving_ground
        .get(&proving_ground.url(HOST, &capture(&page, r#"<iframe [^>]*src="([^"]+)""#)))
        .body;
    json!({
        "id": data["id"],
        "name": data["name"],
        "price": data["price"],
        "description": capture(&frame, r#"<p class="description">([^<]+)</p>"#),
    })
}

/// The Records of every item the feed loads, page after page as it is
/// scrolled to its end.
fn read_feed(proving_ground: &ProvingGround, start_url: &str) -> Vec<Value> {
    let page = proving_ground.get(start_url).body;
    let mut source = Some(capture(&page, r#"<ul class="items" data-source="([^"]+)""#));
    let mut records = Vec::new();
    while let Some(path) = source {
        let feed = proving_ground.get(&proving_ground.url(HOST, &path)).json();
        for item in feed["items"].as_array().expect("a page of items") {
            let url = proving_ground.url(HOST, item["url"].as_str().expect("an item URL"));
            records.push(read_item(proving_ground, &url));
        }
        source = feed["next"].as_str().map(str::to_owned);
    }
    records
}

/// A known-correct result for the task `id`.
fn correct_result(proving_ground: &ProvingGround, id: &str) -> Vec<Value> {
    let task = proving_ground.task(id);
    let start_url = task["start_url"].as_str().expect("a start URL");
    match task["kind"].as_str().expect("a kind") {
        "crawl" => read_feed(proving_ground, start_url),
        _ => vec![read_item(proving_ground, start_url)],
    }
}

/// Covers: story:2-33
#[test]
fn the_feed_shows_no_item_before_its_script_runs() {
    let proving_ground = ProvingGround::start(SEED);
    let task = proving_ground.task("rendered/1");

    let page = proving_ground.get(task["start_url"].as_str().unwrap());

    assert_eq!(page.status, 200, "{}", page.body);
    assert!(!page.body.contains("/items/"), "{}", page.body);
}

/// The app shell's own HTML holds an empty `#app`: its heading is in its
/// inline script only, which a browser runs before the page loads.
///
/// Covers: story:5-137
#[test]
fn the_app_shell_holds_its_heading_only_in_its_script() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&proving_ground.url(HOST, "/shell"));

    let (html, script) = page.body.split_once("<script>").unwrap();
    assert_eq!(
        (
            page.status,
            html.contains(r#"<main id="app"></main>"#),
            html.contains("Rendered shell"),
            script.contains("Rendered shell"),
        ),
        (200, true, false, true),
        "{}",
        page.body
    );
}

/// Covers: story:2-33
#[test]
fn an_item_page_shows_neither_its_name_nor_its_price_before_its_script_runs() {
    let proving_ground = ProvingGround::start(SEED);
    let task = proving_ground.task("rendered/2");
    let start_url = task["start_url"].as_str().unwrap();
    let record = read_item(&proving_ground, start_url);

    let page = proving_ground.get(start_url);

    assert_eq!(page.status, 200, "{}", page.body);
    assert!(
        !page.body.contains(record["name"].as_str().unwrap()),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains(record["price"].as_str().unwrap()),
        "{}",
        page.body
    );
}

/// Covers: story:2-33
#[test]
fn the_feed_loads_twenty_four_items_as_it_is_scrolled() {
    let proving_ground = ProvingGround::start(SEED);
    let task = proving_ground.task("rendered/1");

    let records = read_feed(&proving_ground, task["start_url"].as_str().unwrap());

    assert_eq!(records.len(), 24);
    assert_eq!(records[0]["id"], "I-0001");
    assert_eq!(records[23]["id"], "I-0024");
}

/// Covers: story:2-31, story:2-33
#[rstest]
#[case::crawl("rendered/1")]
#[case::extract("rendered/2")]
fn a_correct_rendered_result_is_verified(#[case] task: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let records = correct_result(&proving_ground, task);

    let answer = proving_ground
        .judge(&submission(task, "success", &records))
        .json();

    assert_eq!(answer["task"], task);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-32, story:2-33
#[test]
fn a_rendered_crawl_missing_one_item_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = correct_result(&proving_ground, "rendered/1");
    let dropped = records.remove(10);

    let answer = proving_ground
        .judge(&submission("rendered/1", "failure", &records))
        .json();

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

/// Covers: story:2-4, story:2-33
#[test]
fn a_crawl_that_never_scrolls_the_feed_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = correct_result(&proving_ground, "rendered/1");
    records.truncate(6);

    let answer = proving_ground
        .judge(&submission("rendered/1", "success", &records))
        .json();

    assert_eq!(answer["task"], "rendered/1");
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"].as_array().unwrap().len(), 18, "{answer}");
}

/// Covers: story:2-4, story:2-33
#[rstest]
#[case::outside_the_shadow_root("name")]
#[case::outside_the_frame("description")]
#[case::before_the_ajax_call("price")]
fn a_success_without_a_rendered_field_is_a_false_success_naming_it(#[case] field: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = correct_result(&proving_ground, "rendered/2");
    let id = records[0]["id"].clone();
    let shown = records[0][field].clone();
    records[0]
        .as_object_mut()
        .expect("a Record is an object")
        .remove(field);

    let answer = proving_ground
        .judge(&submission("rendered/2", "success", &records))
        .json();

    assert_eq!(answer["task"], "rendered/2");
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["wrong"],
        json!([{ "id": id, "field": field, "expected": shown, "submitted": null }])
    );
}
