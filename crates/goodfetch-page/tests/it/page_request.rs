//! `page.request` on an in-process engine's page (ADRs 0020 and 0121): the
//! engine's own page that sends it, in the page's Session, closes with the
//! call, whether it answered or was cancelled.

use goodfetch_recording_gate::RecordingGate;
use serde_json::{Map, json};

use crate::handler_page::{Program, settled_or, with_handler_page};

/// The incidents family's host whose second page answers only after 8
/// seconds.
const SLOW: &str = "6.incidents.test";

/// How many targets the engine closed so far, by the Run trace's command
/// log.
fn closed_targets(program: &Program) -> usize {
    program
        .cdp_commands()
        .iter()
        .filter(|method| *method == "Target.closeTarget")
        .count()
}

/// A `page.request` cancelled while its response is on its way closes the
/// engine's page it opened, as one that answered does, so no tab is left
/// open in the Session's browser.
/// Covers: operation:page.request
#[test]
fn a_cancelled_page_request_closes_the_page_it_opened() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        let before = closed_targets(program);
        let url = program.proving_ground.url(SLOW, "/catalog/2");
        let page = program.page.clone();
        let sent = program
            .request("page.request", &json!({ "page_id": page, "url": url }))
            .await;
        settled_or(
            || program.requested(SLOW, "/catalog/2"),
            || "the slow page was never requested".to_owned(),
        )
        .await;
        program.notify("$/cancel", &json!({ "id": sent })).await;
        let cancelled = program.answer(sent).await;
        settled_or(
            || closed_targets(program) > before,
            || format!("closed targets: {}", closed_targets(program)),
        )
        .await;

        assert_eq!(cancelled["error"]["data"]["code"], json!("call_cancelled"));
    });
}
