//! A locator action waits for the navigation of the page it starts
//! (ADR 0085), until that navigation's document committed, as Playwright's
//! actions do, against real Chrome for Testing and the Proving ground.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_recording_gate::RecordingGate;
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::handler_page::with_handler_page;

/// The navigation test whose page links to `/b`.
const HISTORY: &str = "2.navigation.test";

/// The path of `url`.
fn path_of(url: &Value) -> Option<String> {
    url.as_str()
        .and_then(|url| url::Url::parse(url).ok())
        .map(|url| url.path().to_owned())
}

/// A click on a link, or Enter pressed on it, answers once the navigation
/// it started committed, so the next call acts on the document it reached:
/// going back at once returns to the page the action left. The gate holds
/// that navigation a while before admitting it, as a crawl's politeness
/// delay does, so that it commits well after the input was sent.
/// Covers: operation:page.go_back
#[rstest]
/// Covers: operation:locator.click
#[case::a_click("locator.click", json!({}))]
/// Covers: operation:locator.press
#[case::enter_pressed("locator.press", json!({ "key": "Enter" }))]
fn an_action_answers_once_the_navigation_it_started_committed(
    #[case] operation: &str,
    #[case] params: Value,
) {
    let gate = RecordingGate::new();
    gate.hold("/b", Duration::from_millis(500));
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        let mut params = params;
        params["locator"] = json!({ "page_id": program.page, "selector": "#next" });
        let acted = program.call(operation, &params).await;
        let back = program.on_page("page.go_back", json!({})).await;

        assert_eq!(
            (
                acted.get("result").is_some(),
                path_of(&back["result"]["response"]["url"]),
            ),
            (true, Some("/".to_owned())),
            "{acted} {back}"
        );
    });
}

/// A form whose submit button is clicked, or whose field has Enter pressed
/// in it.
const FORM: &str = "() => {
    document.body.innerHTML = '<form method=\"post\" action=\"/b\"><input id=\"field\" name=\"q\">'
        + '<button id=\"send\" style=\"width:160px;height:40px\">Send</button></form>';
}";

/// A click on a form's submit button, or Enter pressed in its field,
/// answers once the form's navigation committed, so the next call, sent at
/// once, reads the document the form reached (#1766). The gate holds that
/// navigation a while, so that it commits well after the input was sent.
#[rstest]
/// Covers: operation:locator.click
#[case::a_click("locator.click", "#send", json!({}))]
/// Covers: operation:locator.press
#[case::enter_pressed("locator.press", "#field", json!({ "key": "Enter" }))]
fn an_action_that_submits_a_form_answers_once_its_navigation_committed(
    #[case] operation: &str,
    #[case] selector: &str,
    #[case] params: Value,
) {
    let gate = RecordingGate::new();
    gate.hold("/b", Duration::from_millis(500));
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        program
            .on_page("page.evaluate", json!({ "function": FORM }))
            .await;
        let mut params = params;
        params["locator"] = json!({ "page_id": program.page, "selector": selector });
        let acted = program.call(operation, &params).await;
        let path = program
            .on_page(
                "page.evaluate",
                json!({ "function": "() => location.pathname" }),
            )
            .await;

        assert_eq!(
            (acted.get("result").is_some(), &path["result"]["value"]),
            (true, &json!("/b")),
            "{acted} {path}"
        );
    });
}

/// A Run whose `timeout.navigation` is past the harness's own deadline for
/// a call, so an action that waited it out never answers in the test.
fn navigation_past_the_call_deadline() -> Map<String, Value> {
    let mut settings = Map::new();
    settings.insert(
        "timeout".to_owned(),
        json!({ "navigation": 10 * 60 * 1000 }),
    );
    settings
}

/// A click whose input requests no navigation of the page answers without
/// waiting for one: on a button outside any form, on a form's submit button
/// whose `submit` event the page cancels, and on one whose form opens its
/// document in another tab.
/// Covers: operation:locator.click
#[rstest]
#[case::outside_any_form("<button id=\"send\" style=\"width:160px;height:40px\">Send</button>")]
#[case::a_cancelled_submission(
    "<form action=\"/b\" onsubmit=\"event.preventDefault()\"><button id=\"send\" style=\"width:160px;height:40px\">Send</button></form>"
)]
#[case::a_submission_to_another_tab(
    "<form action=\"/b\" target=\"_blank\"><button id=\"send\" style=\"width:160px;height:40px\">Send</button></form>"
)]
fn a_click_that_requests_no_navigation_answers_without_waiting_for_one(#[case] body: &str) {
    let gate = RecordingGate::new();
    with_handler_page(
        &gate,
        navigation_past_the_call_deadline(),
        async |program| {
            program.goto(HISTORY, "/").await;
            program
            .on_page(
                "page.evaluate",
                json!({ "function": "body => { document.body.innerHTML = body; }", "arg": body }),
            )
            .await;
            let page = program.page.clone();
            let acted = program
                .call(
                    "locator.click",
                    &json!({ "locator": { "page_id": page, "selector": "#send" } }),
                )
                .await;
            let path = program
                .on_page(
                    "page.evaluate",
                    json!({ "function": "() => location.pathname" }),
                )
                .await;

            assert_eq!(
                (acted.get("result").is_some(), &path["result"]["value"]),
                (true, &json!("/")),
                "{acted} {path}"
            );
        },
    );
}

/// A dialog a click opens never stalls the click's wait for the navigation
/// it starts: an `alert` the click's own handler opens, which blocks the
/// form's submission until the dialog's default dismisses it, and the
/// `beforeunload` dialog the form's navigation opens, which its default
/// accepts.
/// Covers: operation:locator.click
#[rstest]
#[case::an_alert_of_the_click(
    "document.querySelector('#send').addEventListener('click', () => alert('sent'))"
)]
#[case::a_beforeunload_of_the_navigation(
    "addEventListener('beforeunload', event => { event.preventDefault(); event.returnValue = ''; })"
)]
fn a_dialog_a_click_opens_does_not_stall_its_wait(#[case] script: &str) {
    let gate = RecordingGate::new();
    with_handler_page(
        &gate,
        navigation_past_the_call_deadline(),
        async |program| {
            program.goto(HISTORY, "/").await;
            program
            .on_page(
                "page.evaluate",
                json!({
                    "function": "script => {
                        document.body.innerHTML = '<form action=\"/b\"><button id=\"send\" style=\"width:160px;height:40px\">Send</button></form>';
                        new Function(script)();
                    }",
                    "arg": script,
                }),
            )
            .await;
            let page = program.page.clone();
            let acted = program
                .call(
                    "locator.click",
                    &json!({ "locator": { "page_id": page, "selector": "#send" } }),
                )
                .await;
            let path = program
                .on_page(
                    "page.evaluate",
                    json!({ "function": "() => location.pathname" }),
                )
                .await;

            assert_eq!(
                (acted.get("result").is_some(), &path["result"]["value"]),
                (true, &json!("/b")),
                "{acted} {path}"
            );
        },
    );
}

/// A `beforeunload` dialog a dialog handler dismisses cancels the form's
/// navigation, which ends the click's wait for it: the click answers on the
/// document it left in place.
/// Covers: operation:locator.click
/// Covers: operation:dialog.dismiss
#[test]
fn a_navigation_a_dismissed_beforeunload_cancels_ends_the_clicks_wait() {
    let gate = RecordingGate::new();
    with_handler_page(
        &gate,
        navigation_past_the_call_deadline(),
        async |program| {
            program.goto(HISTORY, "/").await;
            program
                .on_page(
                    "page.evaluate",
                    json!({
                        "function": "() => {
                            document.body.innerHTML = '<form action=\"/b\"><button id=\"send\" style=\"width:160px;height:40px\">Send</button></form>';
                            addEventListener('beforeunload', event => { event.preventDefault(); event.returnValue = ''; });
                        }",
                    }),
                )
                .await;
            let page = program.page.clone();
            program
                .request("page.on_dialog", &json!({ "page_id": page }))
                .await;
            let click = program
                .request(
                    "locator.click",
                    &json!({ "locator": { "page_id": page, "selector": "#send" } }),
                )
                .await;
            let asked = program.until_request("dialog.handle").await;
            let dismissed = program
                .call(
                    "dialog.dismiss",
                    &json!({ "dialog_id": asked["params"]["dialog_id"] }),
                )
                .await;
            program.reply(&asked, json!({})).await;
            let acted = program.answer(click).await;
            let path = program
                .on_page(
                    "page.evaluate",
                    json!({ "function": "() => location.pathname" }),
                )
                .await;

            assert_eq!(
                (
                    &asked["params"]["dialog"]["type"],
                    dismissed.get("result").is_some(),
                    acted.get("result").is_some(),
                    &path["result"]["value"],
                ),
                (&json!("beforeunload"), true, true, &json!("/")),
                "{asked} {dismissed} {acted} {path}"
            );
        },
    );
}
