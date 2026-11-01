//! The guard's interception listener, armed between a pointer target's
//! check and its input (ADR 0019), judges only the browser's own input
//! events: a page's synthetic event elsewhere in that window goes on as
//! the page sent it, as Playwright's listener lets it, against real Chrome
//! for Testing and the Proving ground.

use goodfetch_recording_gate::RecordingGate;
use serde_json::{Map, json};

use crate::handler_page::with_handler_page;

/// The navigation test, whose page the test replaces with its own.
const HISTORY: &str = "2.navigation.test";

/// A click on a button while the page dispatches synthetic clicks on its
/// body without a pause lands once, and none of the page's own events is
/// cancelled.
/// Covers: operation:locator.click
#[test]
fn a_click_cancels_none_of_the_pages_own_synthetic_events() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        program
            .on_page(
                "page.evaluate",
                json!({
                    "function": "() => {
                        document.body.innerHTML = '<button id=\"target\">Go</button>';
                        globalThis.landed = 0;
                        globalThis.cancelled = 0;
                        document.getElementById('target')
                            .addEventListener('click', (event) => { if (event.isTrusted) landed += 1; });
                        globalThis.synthetic = setInterval(() => {
                            const kept = document.body.dispatchEvent(
                                new MouseEvent('click', { bubbles: true, cancelable: true }));
                            if (!kept) cancelled += 1;
                        }, 0);
                    }",
                }),
            )
            .await;
        let page = program.page.clone();

        let clicked = program
            .call(
                "locator.click",
                &json!({ "locator": { "page_id": page, "selector": "#target" } }),
            )
            .await;
        let counted = program
            .on_page(
                "page.evaluate",
                json!({
                    "function": "() => { clearInterval(synthetic); return [landed, cancelled]; }",
                }),
            )
            .await;

        assert_eq!(
            (clicked.get("result").is_some(), &counted["result"]["value"]),
            (true, &json!([1, 0])),
            "{clicked}"
        );
    });
}
