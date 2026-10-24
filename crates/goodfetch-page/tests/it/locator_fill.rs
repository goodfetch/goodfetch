//! `locator.fill` on an input whose value a person picks rather than
//! types, a date, a time, a month, a week, a local date and time, a colour
//! or a range, sets its value and fires `input` and `change`, as
//! Playwright's does; a value the input does not take raises, against real
//! Chrome for Testing and the Proving ground.

use goodfetch_recording_gate::RecordingGate;
use rstest::rstest;
use serde_json::{Map, json};

use crate::handler_page::with_handler_page;

/// The navigation test, whose page the test replaces with its own.
const HISTORY: &str = "2.navigation.test";

/// Replaces the page's body with one input of `kind` that counts its
/// `input` and `change` events.
const FIELD: &str = "(kind) => {
    document.body.innerHTML = `<input id=\"field\" type=\"${kind}\">`;
    globalThis.fired = [];
    const field = document.getElementById('field');
    for (const type of ['input', 'change']) field.addEventListener(type, () => fired.push(type));
}";

/// Covers: operation:locator.fill
#[rstest]
#[case::date("date", "2026-10-24")]
#[case::time("time", "13:45")]
#[case::local_date_and_time("datetime-local", "2026-10-24T13:45")]
#[case::month("month", "2026-10")]
#[case::week("week", "2026-W43")]
#[case::colour("color", "#336699")]
#[case::range("range", "30")]
fn fill_sets_the_value_of_an_input_a_person_picks(#[case] kind: &str, #[case] value: &str) {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        program
            .on_page("page.evaluate", json!({ "function": FIELD, "arg": kind }))
            .await;
        let page = program.page.clone();

        let filled = program
            .call(
                "locator.fill",
                &json!({ "locator": { "page_id": page, "selector": "#field" }, "value": value }),
            )
            .await;
        let read = program
            .on_page(
                "page.evaluate",
                json!({ "function": "() => [document.getElementById('field').value, fired]" }),
            )
            .await;

        assert_eq!(
            (filled.get("result").is_some(), &read["result"]["value"]),
            (true, &json!([value, ["input", "change"]])),
            "{filled}"
        );
    });
}

/// A value a date input does not take raises `invalid_params`, and the
/// input keeps its value.
/// Covers: operation:locator.fill
#[test]
fn fill_raises_for_a_value_a_date_input_does_not_take() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        program
            .on_page("page.evaluate", json!({ "function": FIELD, "arg": "date" }))
            .await;
        let page = program.page.clone();

        let filled = program
            .call(
                "locator.fill",
                &json!({ "locator": { "page_id": page, "selector": "#field" }, "value": "next Tuesday" }),
            )
            .await;
        let read = program
            .on_page(
                "page.evaluate",
                json!({ "function": "() => document.getElementById('field').value" }),
            )
            .await;

        assert_eq!(
            (&filled["error"]["code"], &read["result"]["value"]),
            (&json!(-32602), &json!("")),
            "{filled}"
        );
    });
}
