//! `page.request` on an in-process engine's page (ADRs 0020 and 0121): the
//! engine's own page that sends it, in the page's Session, closes with the
//! call, whether it answered or was cancelled, and none opens once the
//! page's lease ended.

use goodfetch_engine::lease::LeaseEnded;
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

/// How many targets the engine opened so far, by the Run trace's command
/// log.
fn opened_targets(program: &Program) -> usize {
    program
        .cdp_commands()
        .iter()
        .filter(|method| *method == "Target.createTarget")
        .count()
}

/// Once its page's lease ended, a `page.request` raises the lease check's
/// error before the engine opens a page for it: nothing leaves for it, not
/// even the command that would open its page.
/// Covers: operation:page.request
#[test]
fn a_page_request_after_the_lease_ended_opens_no_page() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        gate.end_lease(LeaseEnded::Lost);
        let before = opened_targets(program);
        let url = program.proving_ground.url(SLOW, "/catalog/1");
        let refused = program.on_page("page.request", json!({ "url": url })).await;

        assert_eq!(
            (
                refused["error"]["data"]["code"].clone(),
                opened_targets(program) - before,
                program.requested(SLOW, "/catalog/1"),
            ),
            (json!("call_cancelled"), 0, false)
        );
    });
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
