//! The page suite's Response recording + declared Effects case (ADRs 0105 and
//! 0106, spec #580, ticket #825, `story:6-52`):
//! a request answered from a Response recording inside an Effect's body marks
//! the Effect sent, so the Effect's judgement equals a live Run's while the
//! replaying Run sends zero requests to the Proving ground.

use goodfetch_protocol::effect::{EFFECT_CLOSE, EFFECT_OPEN};
use goodfetch_protocol::page::request::PAGE_REQUEST;
use serde_json::json;

use super::super::navigation::{answered, call};
use super::super::{Checks, Site, new_page, start_run};
use super::{
    CACHE_HOST, WarcBuilder, finish_report, start_with_recording, total_received, write_fixture,
};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The judgement returned by `effect.close` for one Effect scenario.
#[derive(Debug, PartialEq, Eq)]
struct Judgement {
    judged: String,
    sent: Option<bool>,
    reason: Option<String>,
    first_command: Option<String>,
}

/// Runs three Effect scenarios on `run_id`:
/// 1. `page.goto` inside an Effect body, then `threw` -> `uncertain` (`page.goto`);
/// 2. `page.request` inside an Effect body, then `threw` -> `uncertain` (`page.request`);
/// 3. no request inside an Effect body, then `threw` -> `dropped` (`sent: false`).
fn run_effect_scenarios(
    connection: &mut Connection,
    run_id: &str,
    page_url: &str,
    submit_url: &str,
    label: &str,
    checks: &mut Checks,
) -> Result<Option<[Judgement; 3]>, Gone> {
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(None);
    };
    let page_id = json!(page.page_id);

    let goto_judgement = {
        let opened = connection.call(
            EFFECT_OPEN.name,
            &json!({ "run_id": run_id, "key": "eff-goto" }),
        )?;
        let entry = opened["result"]["entry"].as_str().unwrap_or_default();
        let loaded = connection.goto(&page, page_url)?;
        answered(
            &loaded,
            &format!("{label} `goto` inside Effect"),
            page_url,
            200,
            checks,
        );
        close_threw(connection, run_id, entry)?
    };

    let request_judgement = {
        let opened = connection.call(
            EFFECT_OPEN.name,
            &json!({ "run_id": run_id, "key": "eff-request" }),
        )?;
        let entry = opened["result"]["entry"].as_str().unwrap_or_default();
        let resp = call(
            connection,
            PAGE_REQUEST.name,
            &page_id,
            &json!({
                "url": submit_url,
                "method": "POST",
                "headers": { "content-type": "application/json" },
                "post_data": "eyJvcmRlciI6MX0=",
            }),
        )?;
        checks.check(
            format!("{label} `page.request` inside Effect succeeded"),
            resp["result"]["status"] == json!(200),
        );
        close_threw(connection, run_id, entry)?
    };

    let idle_judgement = {
        let opened = connection.call(
            EFFECT_OPEN.name,
            &json!({ "run_id": run_id, "key": "eff-idle" }),
        )?;
        let entry = opened["result"]["entry"].as_str().unwrap_or_default();
        close_threw(connection, run_id, entry)?
    };

    Ok(Some([goto_judgement, request_judgement, idle_judgement]))
}

/// Closes `entry` with `outcome: { ended: "threw" }` and extracts the
/// judgement fields that must match between live and replaying Runs.
fn close_threw(connection: &mut Connection, run_id: &str, entry: &str) -> Result<Judgement, Gone> {
    let closed = connection.call(
        EFFECT_CLOSE.name,
        &json!({
            "run_id": run_id,
            "entry": entry,
            "outcome": { "ended": "threw" },
        }),
    )?;
    let result = &closed["result"];
    Ok(Judgement {
        judged: result["judged"].as_str().unwrap_or_default().to_owned(),
        sent: result["sent"].as_bool(),
        reason: result["effect"]["reason"].as_str().map(str::to_owned),
        first_command: result["effect"]["first_command"]
            .as_str()
            .map(str::to_owned),
    })
}

/// Proves that a request answered by a Response recording inside an Effect's
/// body marks the Effect sent with the exact same judgement as a live Run,
/// while sending zero requests to the Proving ground.
pub(super) fn recorded_request_marks_effect_sent(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let page_url = site.named_url(CACHE_HOST, "/page/effect-live");
    let submit_url = site.named_url(CACHE_HOST, "/api/effect-live");

    // 1. Live Run baseline.
    let live_run = "conformance-effect-live";
    start_run(connection, site, live_run, &json!({}), checks)?;
    let live = run_effect_scenarios(connection, live_run, &page_url, &submit_url, "live", checks)?;
    let _ = finish_report(connection, live_run)?;

    // 2. Replaying Run against a WARC holding the exact same URLs.
    let mut warc = WarcBuilder::new("goodfetch/0.1");
    warc.exchange(
        "2026-01-05T10:00:01Z",
        "GET",
        &page_url,
        &[],
        b"",
        200,
        &[("Content-Type", "text/html")],
        b"<!doctype html><html><body>effect page</body></html>",
    );
    warc.exchange(
        "2026-01-05T10:00:02Z",
        "POST",
        &submit_url,
        &[("Content-Type", "application/json")],
        br#"{"order":1}"#,
        200,
        &[("Content-Type", "application/json")],
        br#"{"accepted":true}"#,
    );
    let warc_path = write_fixture(target, "effect-replay.warc", &warc.finish());

    let before_replay = total_received(site);
    let replay_run = "conformance-effect-replay";
    let started = start_with_recording(connection, site, replay_run, &json!({}), &warc_path)?;
    checks.check(
        "the replaying Run for Effect judgement starts",
        started.get("result").is_some(),
    );
    let replayed = run_effect_scenarios(
        connection,
        replay_run,
        &page_url,
        &submit_url,
        "replayed",
        checks,
    )?;
    let _ = finish_report(connection, replay_run)?;

    checks.check(
        "the replaying Run sent zero requests to the Proving ground",
        total_received(site) == before_replay,
    );
    if let (Some(live), Some(replayed)) = (live, replayed) {
        let expected_goto = Judgement {
            judged: "uncertain".to_owned(),
            sent: None,
            reason: Some("failed_after_send".to_owned()),
            first_command: Some("page.goto".to_owned()),
        };
        let expected_request = Judgement {
            judged: "uncertain".to_owned(),
            sent: None,
            reason: Some("failed_after_send".to_owned()),
            first_command: Some("page.request".to_owned()),
        };
        let expected_idle = Judgement {
            judged: "dropped".to_owned(),
            sent: Some(false),
            reason: None,
            first_command: None,
        };
        checks.check(
            "recorded `page.goto` inside an Effect marks it sent and matches the live Run's judgement",
            live[0] == expected_goto && replayed[0] == live[0],
        );
        checks.check(
            "recorded `page.request` inside an Effect marks it sent and matches the live Run's judgement",
            live[1] == expected_request && replayed[1] == live[1],
        );
        checks.check(
            "an Effect with no request is dropped unsent in both live and replaying Runs",
            live[2] == expected_idle && replayed[2] == live[2],
        );
    }
    Ok(())
}
