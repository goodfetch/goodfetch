//! ADR 0106's send window on an in-process engine's page: a body declared
//! with `effect.open` outside any attempt turns `sent` in the Run's Effect
//! journal when the Run's first command that may change the page goes out,
//! and stays `open` while only reads and waits go out, against real Chrome
//! for Testing and the Proving ground.

use goodfetch_recording_gate::RecordingGate;
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::handler_page::{Program, RUN, with_handler_page};

/// The navigation test, whose page the test replaces with its own.
const HISTORY: &str = "2.navigation.test";

/// Replaces the page's body with a button and an input.
const FORM: &str = "() => {
    document.body.innerHTML = '<button id=\"go\">Go</button><input id=\"field\">';
}";

/// Opens the page's form, then declares the body `order` outside any
/// attempt, which nothing was sent for yet.
async fn declared(program: &mut Program) {
    program.goto(HISTORY, "/").await;
    program
        .on_page("page.evaluate", json!({ "function": FORM }))
        .await;
    let opened = program
        .call("effect.open", &json!({ "run_id": RUN, "key": "order" }))
        .await;
    assert_eq!(opened["result"]["answer"], json!("perform"), "{opened}");
}

/// Reads and waits leave the body `open`: the click after them turns it
/// `sent`, naming the click.
/// Covers: operation:effect.open, operation:locator.click
#[test]
fn a_body_turns_sent_at_the_first_command_that_may_change_the_page() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        declared(program).await;
        let page = program.page.clone();
        let button = json!({ "page_id": page, "selector": "#go" });
        let read = program
            .on_page("page.formats", json!({ "formats": { "markdown": true } }))
            .await;
        let waited = program
            .call(
                "locator.wait_for",
                &json!({ "locator": button, "state": "attached" }),
            )
            .await;
        assert!(
            read.get("result").is_some() && waited.get("result").is_some(),
            "{read} {waited}"
        );
        let clicked = program
            .call("locator.click", &json!({ "locator": button }))
            .await;

        // Only a body still open is marked, with the command that marks
        // it: a read that had marked it would be named instead.
        assert_eq!(
            program.effects(),
            vec![(
                "order".to_owned(),
                "sent".to_owned(),
                Some("locator.click".to_owned())
            )],
            "{clicked}"
        );
    });
}

/// Each command that may change the page marks the body sent with its own
/// name before it goes out.
/// Covers: operation:effect.open
#[rstest]
#[case::click("locator.click", json!({ "locator": { "selector": "#go" } }))]
#[case::fill("locator.fill", json!({ "locator": { "selector": "#field" }, "value": "x" }))]
#[case::press("locator.press", json!({ "locator": { "selector": "#field" }, "key": "a" }))]
#[case::navigation("page.goto", json!({ "url": "" }))]
#[case::evaluate("page.evaluate", json!({ "function": "() => 1" }))]
fn a_command_that_may_change_the_page_marks_the_body_sent(
    #[case] method: &str,
    #[case] params: Value,
) {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        declared(program).await;
        let mut params = params;
        if let Some(locator) = params.get_mut("locator") {
            locator["page_id"] = json!(program.page);
        } else {
            params["page_id"] = json!(program.page);
        }
        if method == "page.goto" {
            params["url"] = json!(program.proving_ground.url(HISTORY, "/"));
        }

        let answered = program.call(method, &params).await;

        assert_eq!(
            program.effects(),
            vec![(
                "order".to_owned(),
                "sent".to_owned(),
                Some(method.to_owned())
            )],
            "{answered}"
        );
    });
}
