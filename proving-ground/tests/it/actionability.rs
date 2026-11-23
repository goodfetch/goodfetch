//! The hand-made `actionability` and `navigation` families, the lab's
//! actionability, primitive, form, navigation, frames and files fixtures
//! ported with their oracle: each page's script posts the event that reached
//! one of its controls, and the oracle requires one trusted event of the right
//! kind on the right control, none on a decoy and none twice, or a refusal
//! where the control under the pointer is not the task's.
//!
//! A page's scripts run only in a browser; these cases post what each script
//! posts, as a plain HTTP client, so they pin the oracle and the pages'
//! contract, never a tool. Expected events are the lab's.

use std::fmt::Write as _;

use rstest::rstest;
use serde_json::{Value, json};

use crate::browserless::{Client, Response};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 161;

fn task(proving_ground: &ProvingGround, id: &str) -> Value {
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .find(|task| task["id"] == id)
        .unwrap_or_else(|| panic!("the task list has no {id}"))
        .clone()
}

fn start_url(task: &Value) -> &str {
    task["start_url"].as_str().expect("a start URL")
}

/// The origin of a URL.
fn origin_of(url: &str) -> String {
    let rest = url.strip_prefix("http://").expect("an http URL");
    format!("http://{}", &rest[..rest.find('/').unwrap_or(rest.len())])
}

/// Posts an event to a task's host name, as its page's script does.
fn post_event(client: &mut Client<'_>, task: &Value, event: &Value) -> Response {
    client.post_json(&format!("{}/effects", origin_of(start_url(task))), event)
}

/// What a tool reports for `task`: its status and its files.
fn result(task: &Value, status: &str, files: &[Value]) -> Value {
    json!({ "task": task["id"], "status": status, "files": files })
}

/// Covers: story:2-29, story:2-40
#[rstest]
#[case::actionability("actionability/1", "<button id=\"target\">")]
#[case::hover("actionability/10", "pointerenter")]
#[case::retarget("actionability/19", "danger")]
#[case::form("actionability/20", "<input type=\"checkbox\" id=\"consent\">")]
#[case::clicks("actionability/26", "<button id=\"counter\">")]
#[case::spa("navigation/1", "history.pushState")]
#[case::download("navigation/7", "download=\"catalog.txt\"")]
fn a_hand_made_task_serves_its_start_page(#[case] id: &str, #[case] control: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, id);
    let mut client = Client::new(&proving_ground);

    let page = client.get(start_url(&task));

    assert_eq!(page.status, 200);
    assert_eq!(
        page.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(page.text().contains(control), "{}", page.text());
}

/// Covers: story:2-29
#[test]
fn every_hand_made_page_loads_the_shared_effect_script() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/10");
    let mut client = Client::new(&proving_ground);

    let script = client.get(&format!("{}/effects.js", origin_of(start_url(&task))));

    assert_eq!(script.status, 200);
    assert_eq!(
        script.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(
        script.text().contains("fetch('/effects'"),
        "{}",
        script.text()
    );
}

/// Covers: story:2-29, story:2-40
#[rstest]
#[case::actionability("actionability/1", json!({ "event": "click", "target": "target", "trusted": true, "details": { "ready": "true" } }))]
#[case::overlay("actionability/2", json!({ "event": "click", "target": "target", "trusted": true, "details": { "ready": "true" } }))]
#[case::hover("actionability/10", json!({ "event": "hover", "target": "target", "trusted": true }))]
#[case::dblclick("actionability/11", json!({ "event": "dblclick", "target": "target", "trusted": true }))]
#[case::contextmenu("actionability/12", json!({ "event": "contextmenu", "target": "target", "trusted": true }))]
#[case::drag("actionability/13", json!({ "event": "drop", "target": "dropzone", "trusted": true, "details": { "moved": "true" } }))]
#[case::wheel("actionability/14", json!({ "event": "scrolled", "target": "scroller", "trusted": true }))]
#[case::keyboard("actionability/15", json!({ "event": "enter", "target": "field", "trusted": true }))]
#[case::contenteditable("actionability/16", json!({ "event": "edited", "target": "editor", "trusted": true, "details": { "text": "Goodfetch" } }))]
#[case::unicode("actionability/17", json!({ "event": "typed", "target": "field", "trusted": true, "details": { "value": "Ünïcødé — 漢字 😀" } }))]
#[case::moved("actionability/18", json!({ "event": "click", "target": "target", "trusted": true }))]
#[case::form("actionability/20", json!({ "event": "submit", "target": "submit", "trusted": true, "details": { "name": "Goodfetch", "region": "eu", "consent": "true", "typed": "true" } }))]
#[case::frame_form("actionability/25", json!({ "event": "submit", "target": "submit", "trusted": true, "details": { "name": "Goodfetch", "region": "eu", "consent": "true" } }))]
#[case::ambiguous_rerender("actionability/27", json!({ "event": "click", "target": "target", "trusted": true, "details": { "ready": "true" } }))]
#[case::controlled("actionability/28", json!({ "event": "submit", "target": "save", "trusted": true, "details": { "state": "Goodfetch", "typed": "true" } }))]
#[case::hover_menu("actionability/29", json!({ "event": "click", "target": "export", "trusted": true, "details": { "hovered": "true" } }))]
#[case::skill_examples("actionability/30", json!({ "event": "search", "target": "q", "trusted": true, "details": { "next": "true", "query": "browser automation" } }))]
#[case::spa("navigation/1", json!({ "event": "route", "target": "go", "trusted": true, "details": { "path": "/detail" } }))]
#[case::history("navigation/2", json!({ "event": "back", "target": "page-a", "trusted": true }))]
#[case::popup("navigation/3", json!({ "event": "click", "target": "confirm", "trusted": true, "details": { "window": "popup" } }))]
#[case::frame("navigation/4", json!({ "event": "click", "target": "confirm", "trusted": true, "details": { "host": "4.navigation.test", "framed": "true" } }))]
#[case::upload("navigation/6", json!({ "event": "upload", "target": "file", "trusted": true, "details": { "name": "upload.txt", "content": "goodfetch-upload\n" } }))]
fn one_trusted_event_of_the_right_kind_on_the_target_is_verified(
    #[case] id: &str,
    #[case] event: Value,
) {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, id);
    let mut client = Client::new(&proving_ground);
    assert_eq!(post_event(&mut client, &task, &event).status, 204);

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn an_untrusted_event_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/11");
    let mut client = Client::new(&proving_ground);
    let untrusted = json!({ "event": "dblclick", "target": "target", "trusted": false });
    post_event(&mut client, &task, &untrusted);

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "event", "event": "dblclick", "target": "target", "trusted": false }])
    );
    assert_eq!(
        answer["missing_effects"],
        json!([{ "kind": "event", "event": "dblclick", "target": "target", "trusted": true }])
    );
}

/// Covers: story:2-40
#[test]
fn a_click_on_the_decoy_the_target_uncovered_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/18");
    let mut client = Client::new(&proving_ground);
    let decoy = json!({ "event": "click", "target": "decoy", "trusted": true });
    post_event(&mut client, &task, &decoy);
    post_event(
        &mut client,
        &task,
        &json!({ "event": "click", "target": "target", "trusted": true }),
    );

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "event", "event": "click", "target": "decoy", "trusted": true }])
    );
}

/// Covers: story:2-40
#[test]
fn an_event_carried_out_twice_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/10");
    let mut client = Client::new(&proving_ground);
    let hover = json!({ "event": "hover", "target": "target", "trusted": true });
    post_event(&mut client, &task, &hover);
    post_event(&mut client, &task, &hover);

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "event", "event": "hover", "target": "target", "trusted": true }])
    );
}

/// Covers: story:2-40
#[test]
fn a_click_before_the_target_is_actionable_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/3");
    let mut client = Client::new(&proving_ground);
    let premature = json!({ "event": "click", "target": "target", "trusted": true, "details": { "ready": "false" } });
    post_event(&mut client, &task, &premature);

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// Covers: story:2-40
#[test]
fn fifty_clicks_one_after_another_are_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/26");
    let mut client = Client::new(&proving_ground);
    for count in 1..=50 {
        let click = json!({ "event": "click", "target": "counter", "trusted": true, "details": { "count": count.to_string() } });
        post_event(&mut client, &task, &click);
    }

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_form_effect_fabricated_without_trusted_events_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/20");
    let mut client = Client::new(&proving_ground);
    let fabricated = json!({ "event": "submit", "target": "submit", "trusted": false, "details": { "name": "Goodfetch", "region": "eu", "consent": "true", "typed": "false" } });
    post_event(&mut client, &task, &fabricated);

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// Covers: story:2-40
#[rstest]
#[case::never_enabled("actionability/7")]
#[case::never_uncovered("actionability/8")]
#[case::ambiguous("actionability/9")]
#[case::retarget("actionability/19")]
fn a_refused_action_that_carried_out_nothing_is_verified(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, id);

    let answer = proving_ground.judge(&result(&task, "failure", &[]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[rstest]
#[case::never_enabled("actionability/7")]
#[case::retarget("actionability/19")]
fn an_action_that_should_have_been_refused_reported_as_a_success_is_a_false_success(
    #[case] id: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, id);

    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_click_on_the_control_swapped_in_under_the_pointer_fails_the_task_even_when_reported_as_a_failure()
 {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/19");
    let mut client = Client::new(&proving_ground);
    let destructive = json!({ "event": "click", "target": "danger", "trusted": true });
    post_event(&mut client, &task, &destructive);

    let answer = proving_ground.judge(&result(&task, "failure", &[]));

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "event", "event": "click", "target": "danger", "trusted": true }])
    );
}

/// Covers: story:2-40
#[test]
fn a_malformed_event_is_refused_and_records_nothing() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "actionability/10");
    let mut client = Client::new(&proving_ground);

    let refused = post_event(
        &mut client,
        &task,
        &json!({ "event": "hover", "kind": "order" }),
    );

    assert_eq!(refused.status, 400);
    let answer = proving_ground.judge(&result(&task, "failure", &[]));
    assert_eq!(answer["extra_effects"], json!([]), "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_cross_origin_frame_comes_from_another_site_whose_events_count_for_the_task() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "navigation/5");
    let mut client = Client::new(&proving_ground);
    let frame = format!("http://5.navigation-frame.test:{}", proving_ground.port());

    let page = client.get(start_url(&task)).text();
    let inner = client.get(&format!("{frame}/inner"));
    let click = json!({ "event": "click", "target": "confirm", "trusted": true, "details": { "host": "5.navigation-frame.test", "framed": "true" } });
    client.post_json(&format!("{frame}/effects"), &click);
    let answer = proving_ground.judge(&result(&task, "success", &[]));

    assert!(
        page.contains(&format!(
            "src=\"//5.navigation-frame.test:{}/inner\"",
            proving_ground.port()
        )),
        "{page}"
    );
    assert_eq!(inner.status, 200);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn the_upload_task_serves_the_file_to_upload() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "navigation/6");
    let mut client = Client::new(&proving_ground);

    let file = client.get(&format!("{}/files/upload.txt", origin_of(start_url(&task))));

    assert_eq!(file.status, 200);
    assert_eq!(file.body, b"goodfetch-upload\n");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02x}").expect("writing to a String succeeds");
        hex
    })
}

/// The bytes the download task serves for seed 161, as the lab's file
/// carried its seed, with a CRLF and bytes no text decoding keeps.
const DOWNLOAD: &[u8] = b"goodfetch-fixture:161\r\n\x00\xff\n";

/// Covers: story:2-40
#[test]
fn a_download_returned_with_its_exact_bytes_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "navigation/7");
    let mut client = Client::new(&proving_ground);

    let file = client.get(&format!(
        "{}/files/catalog.txt",
        origin_of(start_url(&task))
    ));
    let answer = proving_ground.judge(&result(
        &task,
        "success",
        &[json!({ "name": "catalog.txt", "hex": hex(&file.body) })],
    ));

    assert_eq!(file.body, DOWNLOAD);
    assert_eq!(
        file.header("content-disposition"),
        Some("attachment; filename=\"catalog.txt\"")
    );
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_download_whose_line_ends_were_rewritten_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "navigation/7");
    let rewritten = b"goodfetch-fixture:161\n\x00\xff\n";

    let answer = proving_ground.judge(&result(
        &task,
        "success",
        &[json!({ "name": "catalog.txt", "hex": hex(rewritten) })],
    ));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong_files"][0]["name"], "catalog.txt", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_download_under_another_name_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "navigation/7");

    let answer = proving_ground.judge(&result(
        &task,
        "success",
        &[json!({ "name": "download.bin", "hex": hex(DOWNLOAD) })],
    ));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing_files"], json!(["catalog.txt"]), "{answer}");
    assert_eq!(answer["extra_files"], json!(["download.bin"]), "{answer}");
}
