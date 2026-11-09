//! `page.request` on an in-process engine's page (ADRs 0020 and 0121): the
//! engine's own page that sends it, in the page's Session, closes with the
//! call, whether it answered or was cancelled, none opens once the page's
//! lease ended, and the Session's proxy's refusal raises its own code.

use goodfetch_engine::lease::LeaseEnded;
use goodfetch_recording_gate::RecordingGate;
use goodfetch_test_support::ProvingGround;
use serde_json::{Map, json};

use crate::handler_page::{Program, settled_or, with_handler_page, with_handler_page_of};

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

/// A `page.request` through the Session's HTTP proxy that asks for
/// credentials the Session does not have raises `proxy_auth_failed`, as a
/// `page.goto` through it does (ADR 0053): the engine's load answers the
/// proxy's 407 as the page's own requests do.
/// Covers: operation:page.request, error:proxy_auth_failed
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[test]
fn a_page_request_through_a_proxy_refusing_it_raises_proxy_auth_failed() {
    let gate = RecordingGate::new();
    let settings = |proving_ground: &ProvingGround| {
        let proxy = &proving_ground.description().fixtures.proxies["datacenter"];
        let mut settings = Map::new();
        settings.insert(
            "proxy".to_owned(),
            json!({ "server": format!("http://{proxy}") }),
        );
        settings
    };
    with_handler_page_of(&gate, settings, async |program| {
        let url = program
            .proving_ground
            .url("scope.test", "/storage?mark=plain");
        let refused = program.on_page("page.request", json!({ "url": url })).await;

        assert_eq!(
            refused["error"]["data"]["code"],
            json!("proxy_auth_failed"),
            "{refused}"
        );
    });
}
