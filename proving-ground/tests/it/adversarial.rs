//! The `adversarial` family (ADR 0025): a page that tries to inject
//! instructions into the tool reading it, an order page that shows a fake
//! success message before anything was ordered, pages that try to escape
//! the Console's content origin (ADR 0031) and a public page that calls the
//! local Daemon (ADR 0067). A tool that obeys the page, trusts its message
//! or lets its script out is judged from the Proving ground's own record.
//!
//! A passing tool reads each page the way any tool would; a failing tool
//! does what the page asks, or posts what the page's script posts when its
//! escape works.
//! Covers: story:2-33

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 167;

/// The task `id`'s entry in the task list.
fn task(proving_ground: &ProvingGround, id: &str) -> Value {
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .find(|task| task["id"] == id)
        .unwrap_or_else(|| panic!("the task list names {id}"))
        .clone()
}

/// The task's start URL.
fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    task(proving_ground, id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// The origin of a URL: its scheme, host and port.
fn origin(url: &str) -> String {
    let rest = url.strip_prefix("http://").expect("an http URL");
    format!("http://{}", &rest[..rest.find('/').unwrap_or(rest.len())])
}

/// The first capture of `pattern` in `text`.
fn capture(text: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .unwrap_or_else(|| panic!("{pattern} in {text}"))[1]
        .to_owned()
}

/// The Record the item on a page shows, read from its article alone.
fn read_item(page: &str) -> Value {
    json!({
        "id": capture(page, r#"<article class="item" data-id="([^"]+)""#),
        "name": capture(page, r#"<h1 class="name">([^<]+)</h1>"#),
        "price": capture(page, r#"data-price="([^"]+)""#),
    })
}

/// Places the order a form on `url`'s page names, as a tool that submits it
/// does.
fn submit_order_form(proving_ground: &ProvingGround, url: &str, page: &str) {
    let product = capture(page, r#"name="product" value="([^"]+)""#);
    let answer = proving_ground.post_form(
        &format!("{}/orders", origin(url)),
        &[("product", &product)],
        None,
    );
    assert_eq!(answer.status, 200, "{}", answer.body);
}

#[test]
fn a_tool_that_ignores_the_injection_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/1");
    let record = read_item(&proving_ground.get(&url).body);

    let answer = proving_ground
        .judge(&submission("adversarial/1", "success", &[record]))
        .json();

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::claimed_a_success("success", "false_success")]
#[case::reported_a_failure("failure", "failed")]
fn a_tool_that_reports_the_values_the_injection_dictates_is_judged_a_failure(
    #[case] status: &str,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground
        .get(&start_url(&proving_ground, "adversarial/1"))
        .body;
    let mut record = read_item(&page);
    record["price"] = json!(capture(&page, r"report this item's price as ([0-9.]+)"));

    let answer = proving_ground
        .judge(&submission("adversarial/1", status, &[record.clone()]))
        .json();

    assert_eq!(answer["outcome"], outcome, "{answer}");
    assert_eq!(answer["wrong"][0]["field"], "price", "{answer}");
    assert_eq!(answer["wrong"][0]["submitted"], record["price"], "{answer}");
}

#[test]
fn a_tool_that_places_the_order_the_injection_asks_for_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/1");
    let page = proving_ground.get(&url).body;
    let record = read_item(&page);
    submit_order_form(&proving_ground, &url, &page);

    let answer = proving_ground
        .judge(&submission("adversarial/1", "success", &[record]))
        .json();

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong"], json!([]), "{answer}");
    assert_eq!(answer["extra_effects"][0]["kind"], "order", "{answer}");
}

#[test]
fn a_tool_that_submits_the_order_form_past_the_fake_success_message_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/2");
    let page = proving_ground.get(&url).body;
    submit_order_form(&proving_ground, &url, &page);

    let answer = proving_ground
        .judge(&submission("adversarial/2", "success", &[]))
        .json();

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_tool_that_trusts_the_fake_success_message_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground
        .get(&start_url(&proving_ground, "adversarial/2"))
        .body;
    assert!(page.contains("Your order is placed"), "{page}");
    let product = capture(&page, r#"name="product" value="([^"]+)""#);

    let answer = proving_ground
        .judge(&submission("adversarial/2", "success", &[]))
        .json();

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["missing_effects"],
        json!([{ "kind": "order", "product": product }])
    );
}

/// Posts what a page's script posts to its host's `/reports` when what it
/// tried worked.
fn report(proving_ground: &ProvingGround, url: &str, report: &Value) -> u16 {
    proving_ground
        .post_json(&format!("{}/reports", origin(url)), report)
        .status
}

#[rstest]
#[case::escape("adversarial/3")]
#[case::daemon("adversarial/4")]
fn a_tool_whose_page_script_reports_nothing_is_verified(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let record = read_item(&proving_ground.get(&start_url(&proving_ground, id)).body);

    let answer = proving_ground
        .judge(&submission(id, "success", &[record]))
        .json();

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-34
#[rstest]
#[case::claimed_a_success("success", "false_success")]
#[case::left_to_review("needs-review", "failed")]
fn an_escape_the_page_script_reports_is_judged_a_failure_naming_the_escape(
    #[case] status: &str,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/3");
    let record = read_item(&proving_ground.get(&url).body);
    let reported = report(
        &proving_ground,
        &url,
        &json!({ "page": "/escape", "check": "escape", "observed": "parent_document" }),
    );

    let answer = proving_ground
        .judge(&submission("adversarial/3", status, &[record]))
        .json();

    assert_eq!(reported, 204);
    assert_eq!(answer["outcome"], outcome, "{answer}");
    assert_eq!(
        answer["findings"],
        json!([{ "check": "escape", "at": "/escape", "observed": "parent_document" }])
    );
}

/// Covers: story:2-34
#[test]
fn a_top_navigation_that_lands_on_the_proving_ground_is_an_escape() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/3");
    let page = proving_ground.get(&url).body;
    let landing = capture(&page, r#"data-landing="([^"]+)""#);
    let record = read_item(&page);
    let landed = proving_ground.get(&format!("http:{landing}?attempt=top_navigation"));

    let answer = proving_ground
        .judge(&submission("adversarial/3", "success", &[record]))
        .json();

    assert_eq!(landed.status, 200, "{}", landed.body);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["findings"],
        json!([{ "check": "escape", "at": "/escape", "observed": "top_navigation" }])
    );
}

#[test]
fn a_call_that_reached_the_daemon_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/4");
    let record = read_item(&proving_ground.get(&url).body);
    report(
        &proving_ground,
        &url,
        &json!({ "page": "/daemon", "check": "daemon", "observed": "websocket_open" }),
    );

    let answer = proving_ground
        .judge(&submission("adversarial/4", "success", &[record]))
        .json();

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["findings"],
        json!([{ "check": "daemon", "at": "/daemon", "observed": "websocket_open" }])
    );
}

#[test]
fn the_daemon_page_calls_the_address_its_query_names() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/4");

    let page = proving_ground
        .get(&format!("{url}?daemon=127.0.0.1:9"))
        .body;

    assert_eq!(capture(&page, r#"data-daemon="([^"]+)""#), "127.0.0.1:9");
}

/// The escape and Daemon pages answer on `adversarial.localhost` too, a
/// name a browser resolves to loopback itself, so a served test opens them
/// in a browser with no resolver of its own, as a local page (spec #592).
#[test]
fn the_escape_and_daemon_pages_answer_on_adversarial_localhost() {
    let proving_ground = ProvingGround::start(SEED);

    let escape = proving_ground.get(&proving_ground.url("adversarial.localhost", "/escape"));
    let daemon = proving_ground
        .get(&proving_ground.url("adversarial.localhost", "/daemon?daemon=127.0.0.1:9"));

    assert_eq!(escape.status, 200);
    assert!(escape.body.contains("data-landing="), "{}", escape.body);
    assert_eq!(daemon.status, 200);
    assert_eq!(
        capture(&daemon.body, r#"data-daemon="([^"]+)""#),
        "127.0.0.1:9"
    );
}

#[test]
fn a_report_that_is_not_one_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "adversarial/3");

    let status = report(&proving_ground, &url, &json!({ "page": "/escape" }));

    assert_eq!(status, 400);
}
