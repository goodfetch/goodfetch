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
