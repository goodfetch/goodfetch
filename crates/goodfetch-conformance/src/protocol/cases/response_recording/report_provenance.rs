//! The page suite's Response recording Run report and Provenance case
//! (ADR 0105, ADR 0140; spec #580, ticket #802):
//! - the Run report's `recording` holds mode, file, responses used,
//!   `recorded_from`, `recorded_to`, `written_by` and every miss;
//! - responses answered from the recording are marked in Provenance with
//!   `origin.kind == "response_recording"` and their `recorded_at` time;
//! - a recording written years ago produces the same fields and no failure
//!   or age warning.

use std::time::Duration;

use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::run::{RESPONSE_NOT_RECORDED, RUN_START};
use serde_json::{Value, json};

use super::super::navigation::call;
use super::super::{Checks, Site, new_page, poll, run_trace};
use super::{HOST, RECORDED_WITHIN, WarcBuilder, finish_report, total_received, write_fixture};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

const OLD_FROM: &str = "2019-03-10T08:15:00Z";
const OLD_TO: &str = "2019-03-11T14:45:00Z";
const OLD_WRITER: &str = "goodfetch-collie/0.1";

/// Builds the 2019 WARC fixture holding `/old-report-page` and `/old-report-data`.
fn build_old_recording(
    target: &EngineTarget,
    page_url: &str,
    data_url: &str,
) -> std::path::PathBuf {
    let html = format!(
        r#"<!doctype html><html><head><link rel="icon" href="data:,"></head><body>
<p id="out">pending</p>
<script>
fetch({data_url_json}).then(r => r.json()).then(d => {{
  document.getElementById("out").textContent = d.age;
}});
</script></body></html>"#,
        data_url_json = serde_json::to_string(data_url).expect("url serializes")
    );
    let mut builder = WarcBuilder::new(OLD_WRITER);
    builder.exchange(
        OLD_FROM,
        "GET",
        page_url,
        &[("Host", HOST)],
        b"",
        200,
        &[("Content-Type", "text/html; charset=utf-8")],
        html.as_bytes(),
    );
    builder.exchange(
        OLD_TO,
        "GET",
        data_url,
        &[("Host", HOST)],
        b"",
        200,
        &[("Content-Type", "application/json")],
        br#"{"age":"2019"}"#,
    );
    write_fixture(target, "old-recording-2019.warc", &builder.finish())
}

/// Checks `RunReport.recording`, no age warning, and Provenance entries.
fn verify_report_and_provenance(
    target: &EngineTarget,
    run_id: &str,
    warc_path: &std::path::Path,
    (page_url, data_url, missing_url): (&str, &str, &str),
    finished: &Value,
    checks: &mut Checks,
) {
    let report = &finished["result"];
    let rec = &report["recording"];
    let expected_miss = format!("GET {missing_url}");
    let unexpected_warnings = report["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| warning["code"] != json!("clean_up_rules_stale"))
        .count();
    checks.check(
        "old recording finishes with no error and no age warning",
        report["lifecycle"] == json!("finished")
            && report.get("error").is_none()
            && unexpected_warnings == 0,
    );
    checks.check(
        "Run report `recording` carries mode, file, responses_used, recorded_from, recorded_to, written_by and misses",
        rec["mode"] == json!("replay")
            && rec["file"] == json!(warc_path.to_string_lossy())
            && rec["responses_used"] == json!(2)
            && rec["recorded_from"] == json!(OLD_FROM)
            && rec["recorded_to"] == json!(OLD_TO)
            && rec["written_by"] == json!(OLD_WRITER)
            && rec["misses"] == json!([expected_miss]),
    );

    let entries = checks.run_trace(poll::try_until(
        RECORDED_WITHIN,
        Duration::from_millis(100),
        || {
            Ok(run_trace::provenance(target, run_id)?
                .into_iter()
                .filter(|entry| entry["action"] == json!("response_recording"))
                .collect::<Vec<_>>())
        },
        |list| list.len() >= 2,
    ));
    let has_page = entries.iter().any(|entry| {
        entry["url"] == json!(page_url)
            && entry["origin"]["kind"] == json!("response_recording")
            && entry["origin"]["recorded_at"] == json!(OLD_FROM)
    });
    let has_data = entries.iter().any(|entry| {
        entry["url"] == json!(data_url)
            && entry["origin"]["kind"] == json!("response_recording")
            && entry["origin"]["recorded_at"] == json!(OLD_TO)
    });
    checks.check(
        "Provenance marks both replayed responses with `origin.kind == \"response_recording\"` and their `recorded_at`",
        has_page && has_data,
    );
}

/// Case 11 (`story:6-60`): a recording written long ago replays with no
/// failure or age warning, reports all fields of `RunReport.recording`, and
/// marks every replayed response in Provenance.
pub(super) fn report_and_provenance_of_old_recording(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let page_url = site.named_url(HOST, "/old-report-page");
    let data_url = site.named_url(HOST, "/old-report-data");
    let missing_url = site.named_url(HOST, "/old-report-missing");
    let warc_path = build_old_recording(target, &page_url, &data_url);

    let before = total_received(site);
    let run_id = "conformance-rec-old-report";
    let start_id = connection.request(
        RUN_START.name,
        &json!({
            "run_id": run_id,
            "settings": { "network": { "resolve": {} } },
            "recording": { "mode": "replay", "path": warc_path.to_string_lossy() },
        }),
    )?;
    let started = connection.answer(start_id)?.message;
    checks.check(
        "old 2019 recording starts cleanly",
        started.get("result").is_some(),
    );
    let Some(page) = new_page(connection, run_id, checks)? else {
        let _ = finish_report(connection, run_id);
        return Ok(());
    };
    let page_id = json!(page.page_id);
    let went = connection.goto(&page, &page_url)?;
    let out = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({
            "function": r#"() => new Promise((resolve) => {
                const check = () => {
                    const text = document.getElementById("out").textContent;
                    if (text !== "pending") resolve(text);
                    else setTimeout(check, 10);
                };
                check();
            })"#,
        }),
    )?;
    checks.check(
        "old recording answers both document and subresource without network",
        went.get("result").is_some()
            && out["result"]["value"] == json!("2019")
            && total_received(site) == before,
    );

    let missed = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": missing_url }),
    )?;
    checks.check(
        "missing request raises `response_not_recorded`",
        missed["error"]["data"]["code"] == json!(RESPONSE_NOT_RECORDED.name),
    );

    let finished = finish_report(connection, run_id)?;
    verify_report_and_provenance(
        target,
        run_id,
        &warc_path,
        (&page_url, &data_url, &missing_url),
        &finished,
        checks,
    );
    Ok(())
}
