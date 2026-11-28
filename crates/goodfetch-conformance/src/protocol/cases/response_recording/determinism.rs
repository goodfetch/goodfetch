//! The page suite's Response recording determinism and loopback-bypass cases
//! (ADRs 0059, 0073 and 0105; spec #580, ticket #801):
//! - a Proving ground page printing the current date and `Math.random()`
//!   renders the same on two replays and as recorded, and `Date` keeps
//!   running forward after it starts; a response without `WARC-Date` falls
//!   back to its HTTP `Date` header;
//! - a host mapped to loopback by `network.resolve` is reached live during a
//!   replay while a second host is answered from the recording without
//!   reaching the Proving ground.

use std::path::Path;

use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::run::RUN_START;
use serde_json::{Value, json};

use super::super::navigation::call;
use super::super::{Checks, Site, new_page};
use super::record::start_with_mode;
use super::{HOST as REPLAYED_HOST, WarcBuilder, finish_report, total_received, write_fixture};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The Proving ground's rendered family host serving `/clock-random` and `/shell`.
const RENDERED_HOST: &str = "1.rendered.test";

/// What `/clock-random` printed into its DOM.
#[derive(Debug, PartialEq, Eq)]
struct ClockSample {
    date: String,
    epoch: i64,
    random: String,
    random2: String,
    later_epoch: i64,
}

/// Navigates a page of `run_id` to `url` and reads its `#date`, `#epoch`,
/// `#random`, `#random2`, and a `Date.now()` sample taken 40 ms later.
fn read_clock_sample(
    connection: &mut Connection,
    run_id: &str,
    url: &str,
    checks: &mut Checks,
) -> Result<Option<ClockSample>, Gone> {
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(None);
    };
    let page_id = json!(page.page_id);
    let went = connection.goto(&page, url)?;
    if went.get("result").is_none() {
        return Ok(None);
    }
    let eval = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({
            "function": r#"() => new Promise((resolve) => {
                const date = document.getElementById("date").textContent;
                const epoch = Number(document.getElementById("epoch").textContent);
                const random = document.getElementById("random").textContent;
                const random2 = document.getElementById("random2").textContent;
                setTimeout(() => resolve({ date, epoch, random, random2, later_epoch: Date.now() }), 40);
            })"#,
        }),
    )?;
    let value = &eval["result"]["value"];
    let Some(date) = value["date"].as_str() else {
        return Ok(None);
    };
    let Some(epoch) = value["epoch"].as_i64() else {
        return Ok(None);
    };
    let Some(random) = value["random"].as_str() else {
        return Ok(None);
    };
    let Some(random2) = value["random2"].as_str() else {
        return Ok(None);
    };
    let Some(later_epoch) = value["later_epoch"].as_i64() else {
        return Ok(None);
    };
    Ok(Some(ClockSample {
        date: date.to_owned(),
        epoch,
        random: random.to_owned(),
        random2: random2.to_owned(),
        later_epoch,
    }))
}

/// Case 9 (`story:6-58`): a Proving ground page that prints the current date
/// and random numbers renders the same on two replays and as recorded, `Date`
/// keeps running after it starts, and a response without `WARC-Date` starts
/// `Date` from its HTTP `Date` header.
pub(super) fn replay_clock_and_randomness(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let warc_path = write_fixture(target, "clock-random.warc", b"");
    let url = site.named_url(RENDERED_HOST, "/clock-random");

    // 1. Record the live Proving ground page.
    let record_run = "conformance-rec-clock-record";
    let started = start_with_mode(
        connection,
        site,
        record_run,
        &json!({}),
        "record",
        &warc_path,
    )?;
    checks.check(
        "`run.start` in `record` mode succeeds for `/clock-random`",
        started.get("result").is_some(),
    );
    let recorded = read_clock_sample(connection, record_run, &url, checks)?;
    let _ = finish_report(connection, record_run)?;
    let Some(recorded) = recorded else {
        checks.check("recorded `/clock-random` sample was read", false);
        return Ok(());
    };

    // 2. Replay twice with no network access.
    let before = total_received(site);
    let mut samples = Vec::with_capacity(2);
    for (idx, run_id) in [
        "conformance-rec-clock-replay-1",
        "conformance-rec-clock-replay-2",
    ]
    .into_iter()
    .enumerate()
    {
        let started = start_with_mode(connection, site, run_id, &json!({}), "replay", &warc_path)?;
        checks.check(
            format!("replay {} starts cleanly", idx + 1),
            started.get("result").is_some(),
        );
        let sample = read_clock_sample(connection, run_id, &url, checks)?;
        let _ = finish_report(connection, run_id)?;
        if let Some(sample) = sample {
            samples.push(sample);
        }
    }
    checks.check(
        "both replays read `/clock-random` without reaching the Proving ground",
        samples.len() == 2 && total_received(site) == before,
    );
    if samples.len() == 2 {
        let [first, second] = [&samples[0], &samples[1]];
        checks.check(
            "replayed date matches the recorded date on both replays",
            first.date == recorded.date && second.date == recorded.date,
        );
        checks.check(
            "replayed `Math.random()` values are identical on both replays",
            first.random == second.random
                && first.random2 == second.random2
                && first.random != first.random2,
        );
        checks.check(
            "replayed `Date` keeps running forward after starting at the recorded time",
            first.later_epoch > first.epoch && second.later_epoch > second.epoch,
        );
    }

    // 3. A response with no `WARC-Date` falls back to its HTTP `Date` header.
    replay_http_date_header_fallback(connection, target, site, checks)
}

/// Verifies that a recorded response without `WARC-Date` starts `Date` at its
/// HTTP `Date` header and keeps running forward.
fn replay_http_date_header_fallback(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let header_url = site.named_url(REPLAYED_HOST, "/date-header-clock");
    let html = r#"<!doctype html><html><head><link rel="icon" href="data:,"></head><body>
<p id="date"></p><p id="epoch"></p><p id="random"></p><p id="random2"></p>
<script>
const now = new Date();
document.getElementById("date").textContent = now.toISOString().slice(0, 10);
document.getElementById("epoch").textContent = String(now.getTime());
document.getElementById("random").textContent = String(Math.random());
document.getElementById("random2").textContent = String(Math.random());
</script></body></html>"#;
    let mut builder = WarcBuilder::new("goodfetch-collie/0.1");
    builder.exchange_with_optional_date(
        None,
        "GET",
        &header_url,
        &[("Host", REPLAYED_HOST)],
        b"",
        200,
        &[
            ("Content-Type", "text/html; charset=utf-8"),
            ("Date", "Wed, 15 Jun 2022 08:30:00 GMT"),
        ],
        html.as_bytes(),
    );
    let header_warc = write_fixture(target, "date-header-clock.warc", &builder.finish());
    let header_run = "conformance-rec-date-header";
    let started = start_with_mode(
        connection,
        site,
        header_run,
        &json!({}),
        "replay",
        &header_warc,
    )?;
    checks.check(
        "replay of HTTP `Date` header fixture starts cleanly",
        started.get("result").is_some(),
    );
    let header_sample = read_clock_sample(connection, header_run, &header_url, checks)?;
    let _ = finish_report(connection, header_run)?;
    checks.check(
        "response without `WARC-Date` starts `Date` at its HTTP `Date` header and keeps running",
        header_sample.is_some_and(|sample| {
            sample.date == "2022-06-15"
                && (1_655_281_800_000..1_655_281_860_000).contains(&sample.epoch)
                && sample.later_epoch > sample.epoch
        }),
    );
    Ok(())
}

/// Starts a replay Run whose `network.resolve` maps ONLY `live_host` to the
/// Proving ground's loopback address while leaving every other host unmapped.
fn start_replay_with_live_host(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    live_host: &str,
    path: &Path,
) -> Result<Value, Gone> {
    let mut settings = site.mapped(&json!({}), connection.acts_for_model());
    let live_addr = settings["network"]["resolve"][live_host].clone();
    settings["network"]["resolve"] = json!({ live_host: live_addr });
    let id = connection.request(
        RUN_START.name,
        &json!({
            "run_id": run_id,
            "settings": settings,
            "recording": { "mode": "replay", "path": path.to_string_lossy() },
        }),
    )?;
    Ok(connection.answer(id)?.message)
}

/// Case 10 (`story:6-59`): a host mapped by `network.resolve` to loopback is
/// reached live during a replay while a second host is answered from the WARC
/// file without reaching the Proving ground.
pub(super) fn loopback_resolve_bypass_during_replay(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let live_url = site.named_url(RENDERED_HOST, "/shell");
    let replayed_url = site.named_url(REPLAYED_HOST, "/replayed-only");

    let mut builder = WarcBuilder::new("goodfetch-collie/0.1");
    builder.exchange(
        "2026-01-05T09:00:00Z",
        "GET",
        &replayed_url,
        &[("Host", REPLAYED_HOST)],
        b"",
        200,
        &[("Content-Type", "text/html; charset=utf-8")],
        b"<!doctype html><html><head><link rel=\"icon\" href=\"data:,\"></head><body><p id=\"marker\">from-warc-only</p></body></html>",
    );
    let warc_path = write_fixture(target, "loopback-bypass.warc", &builder.finish());

    let before_live = site
        .received_requests()
        .every()
        .filter(|request| request.host == RENDERED_HOST && request.target == "/shell")
        .count();
    let before_replayed = site
        .received_requests()
        .every()
        .filter(|request| request.host == REPLAYED_HOST)
        .count();

    let run_id = "conformance-rec-loopback-bypass";
    let started = start_replay_with_live_host(connection, site, run_id, RENDERED_HOST, &warc_path)?;
    checks.check(
        "replay Run with one loopback-mapped host starts cleanly",
        started.get("result").is_some(),
    );
    let Some(page) = new_page(connection, run_id, checks)? else {
        let _ = finish_report(connection, run_id);
        return Ok(());
    };
    let page_id = json!(page.page_id);

    // 1. Loopback-mapped host reaches the live Proving ground.
    let live_goto = connection.goto(&page, &live_url)?;
    let live_text = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({ "function": "() => document.querySelector('#app .title')?.textContent ?? ''" }),
    )?;
    let after_live = site
        .received_requests()
        .every()
        .filter(|request| request.host == RENDERED_HOST && request.target == "/shell")
        .count();
    checks.check(
        "host mapped by `resolve` to loopback is reached live during replay",
        live_goto.get("result").is_some()
            && live_text["result"]["value"] == json!("Rendered shell")
            && after_live == before_live + 1,
    );

    // 2. Unmapped host is answered from the WARC without touching the Proving ground.
    let replayed_goto = connection.goto(&page, &replayed_url)?;
    let replayed_text = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({ "function": "() => document.getElementById('marker')?.textContent ?? ''" }),
    )?;
    let after_replayed = site
        .received_requests()
        .every()
        .filter(|request| request.host == REPLAYED_HOST)
        .count();
    checks.check(
        "unmapped host is answered from the WARC without reaching the Proving ground",
        replayed_goto.get("result").is_some()
            && replayed_text["result"]["value"] == json!("from-warc-only")
            && after_replayed == before_replayed,
    );

    let finished = finish_report(connection, run_id)?;
    checks.check(
        "Run report records 1 response used from the WARC and no misses",
        finished["result"]["recording"]["responses_used"] == json!(1)
            && finished["result"]["recording"]
                .get("misses")
                .is_none_or(|misses| misses == &json!([])),
    );
    Ok(())
}
