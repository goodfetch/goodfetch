//! A pending `goto` that a navigation the page started itself supersedes
//! before it committed raises `navigation_interrupted` (ADR 0085; spec
//! #594's P17), never a retryable `navigation_failed`, against real Chrome
//! for Testing and the Proving ground.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_recording_gate::RecordingGate;
use serde_json::{Map, json};

use crate::handler_page::with_handler_page;

/// The navigation test whose page links to `/b`.
const HISTORY: &str = "2.navigation.test";

/// The page's own navigation, to its first page again, starts while the
/// gate holds the program's `goto` to `/b`. The page holds a click's user
/// activation: without one, Chromium ignores a navigation the page starts
/// while one the browser started is pending, and `goto` reaches `/b`.
/// Covers: error:navigation_interrupted, operation:page.goto
#[test]
fn a_goto_the_pages_own_navigation_supersedes_is_interrupted() {
    let gate = RecordingGate::new();
    gate.hold("/b", Duration::from_secs(3));
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        let page = program.page.clone();
        program
            .call(
                "locator.click",
                &json!({ "locator": { "page_id": page, "selector": "body" } }),
            )
            .await;
        let own = program.proving_ground.url(HISTORY, "/?own");
        program
            .on_page(
                "page.evaluate",
                json!({
                    "function": "(url) => { setTimeout(() => { location.href = url; }, 500); }",
                    "arg": own,
                }),
            )
            .await;
        let held = program.proving_ground.url(HISTORY, "/b");

        let went = program.on_page("page.goto", json!({ "url": held })).await;

        assert_eq!(
            (
                &went["error"]["data"]["code"],
                &went["error"]["data"]["retryable"]
            ),
            (&json!("navigation_interrupted"), &json!(false)),
            "{went}"
        );
    });
}
