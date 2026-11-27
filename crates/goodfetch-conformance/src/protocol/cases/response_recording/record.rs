//! The page suite's Response recording `mode: "record"` cases (ADR 0105,
//! spec #580, ticket #800):
//! - atomic replacement at Run end, `warcinfo` engine/Run fields, and
//!   `record` -> `replay` round-trip with zero network requests;
//! - redaction of `Cookie`, `Authorization`, `Proxy-Authorization`, masked
//!   `Set-Cookie` values, and Secret canary search with zero hits;
//! - cut response recorded with `WARC-Truncated: disconnect` and replayed as
//!   `response_truncated`;
//! - `record` Run reading no cache entry, storing into cache, and merging both
//!   halves after a kill and resume.

use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::page::RESPONSE_TRUNCATED;
use goodfetch_protocol::page::events::RESPONSE_BODY;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::run::RUN_START;
use serde_json::{Value, json};

use super::super::fetch_tiers::cached::HOST as CACHE_HOST;
use super::super::navigation::call;
use super::super::{Checks, Site, new_page};
use super::{finish_report, request_body, total_received, write_fixture};
use crate::canaries::held_under;
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::hello_as;
use crate::protocol::target::EngineTarget;

/// The Proving ground's task whose login sets a Session cookie canary.
const COOKIE_TASK: &str = "canaries/3";

/// Starts `run_id` with `recording: { mode, path }` under `settings`.
/// In `mode: "replay"`, clears `network.resolve` so replayed hosts are not
/// treated as live loopback-mapped hosts (ADR 0105, issue 801).
pub(super) fn start_with_mode(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    mode: &str,
    path: &Path,
) -> Result<Value, Gone> {
    let mut settings = site.mapped(settings, connection.acts_for_model());
    if mode == "replay" {
        settings["network"]["resolve"] = json!({});
    }
    let id = connection.request(
        RUN_START.name,
        &json!({
            "run_id": run_id,
            "settings": settings,
            "recording": { "mode": mode, "path": path.to_string_lossy() },
        }),
    )?;
    Ok(connection.answer(id)?.message)
}

/// Case 5 (`story:6-55`): a `record` Run replaces its WARC atomically at Run
/// end, names engine and Run in `warcinfo`, and replays with identical results
/// and zero network requests.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous conformance harness reads the recorded WARC file on its own thread"
)]
pub(super) fn record_atomic_and_replay(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let warc_path = write_fixture(target, "recorded-roundtrip.warc", b"SENTINEL-BEFORE-END");
    let record_run = "conformance-rec-write-1";
    let page_url = site.named_url(CACHE_HOST, "/page/rec-roundtrip");
    let post_url = site.named_url(CACHE_HOST, "/api/rec-roundtrip");

    let started = start_with_mode(
        connection,
        site,
        record_run,
        &json!({}),
        "record",
        &warc_path,
    )?;
    checks.check(
        "`run.start` with `recording: { mode: \"record\" }` succeeds",
        started.get("result").is_some(),
    );
    let Some(page) = new_page(connection, record_run, checks)? else {
        return Ok(());
    };
    let page_id = json!(page.page_id);
    let went = connection.goto(&page, &page_url)?;
    checks.check(
        "live goto during record succeeds",
        went.get("result").is_some(),
    );
    let live_text = eval_text(connection, &page_id)?;
    let post_answer = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({
            "url": post_url,
            "method": "POST",
            "headers": { "content-type": "application/json" },
            "post_data": data_encoding::BASE64.encode(br#"{"item":"book"}"#),
        }),
    )?;
    let live_post_body = request_body(connection, &page_id, &post_answer)?;

    // Before the Run ends, the existing file has NOT been overwritten yet.
    let before_end = std::fs::read(&warc_path).unwrap_or_default();
    checks.check(
        "existing recording file is untouched until the Run ends",
        before_end == b"SENTINEL-BEFORE-END",
    );

    let finished = finish_report(connection, record_run)?;
    let report_rec = &finished["result"]["recording"];
    checks.check(
        "Run report records mode `record`, writing engine, time span and responses",
        report_rec["mode"] == json!("record")
            && report_rec["written_by"].is_string()
            && report_rec["recorded_from"].is_string()
            && report_rec["recorded_to"].is_string()
            && report_rec["responses_used"]
                .as_u64()
                .is_some_and(|n| n >= 2),
    );

    let warc_text =
        String::from_utf8_lossy(&std::fs::read(&warc_path).unwrap_or_default()).into_owned();
    checks.check(
        "written WARC replaces the sentinel and carries warcinfo naming engine, Run, mode and writer version",
        warc_text.starts_with("WARC/1.1\r\n")
            && warc_text.contains("WARC-Type: warcinfo")
            && warc_text.contains(&format!("run-id: {record_run}"))
            && warc_text.contains("software: ")
            && warc_text.contains("mode: record")
            && warc_text.contains("warc-writer-version: "),
    );

    // Now replay the recorded WARC and verify identical results and zero network hits.
    verify_replay_matches_live(
        connection,
        site,
        &warc_path,
        (&page_url, &post_url),
        (&live_text, live_post_body.as_deref()),
        checks,
    )
}

fn verify_replay_matches_live(
    connection: &mut Connection,
    site: &Site,
    warc_path: &Path,
    (page_url, post_url): (&str, &str),
    (live_text, live_post_body): (&Value, Option<&[u8]>),
    checks: &mut Checks,
) -> Result<(), Gone> {
    let before_replay = total_received(site);
    let replay_run = "conformance-rec-replay-recorded-1";
    let replayed = start_with_mode(
        connection,
        site,
        replay_run,
        &json!({}),
        "replay",
        warc_path,
    )?;
    checks.check(
        "replay of recorded file starts",
        replayed.get("result").is_some(),
    );
    let Some(rpage) = new_page(connection, replay_run, checks)? else {
        return Ok(());
    };
    let rpage_id = json!(rpage.page_id);
    let rwent = connection.goto(&rpage, page_url)?;
    checks.check("replayed goto succeeds", rwent.get("result").is_some());
    let replay_text = eval_text(connection, &rpage_id)?;
    let rpost = call(
        connection,
        PAGE_REQUEST.name,
        &rpage_id,
        &json!({
            "url": post_url,
            "method": "POST",
            "headers": { "content-type": "application/json" },
            "post_data": data_encoding::BASE64.encode(br#"{"item":"book"}"#),
        }),
    )?;
    let replay_post_body = request_body(connection, &rpage_id, &rpost)?;
    let rfinished = finish_report(connection, replay_run)?;
    let no_misses = rfinished["result"]["recording"]
        .get("misses")
        .is_none_or(|m| m == &json!([]));
    checks.check(
        "replayed page and POST match recorded results with zero misses and zero network requests",
        &replay_text == live_text
            && replay_post_body.as_deref() == live_post_body
            && no_misses
            && total_received(site) == before_replay,
    );
    Ok(())
}

/// Case 6 (`story:6-56`): a recorded WARC holds no `Cookie`, `Authorization`,
/// or `Proxy-Authorization` header, no `Set-Cookie` value and no Secret canary.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous conformance harness reads the recorded WARC file on its own thread"
)]
pub(super) fn record_redacts_and_masks_canaries(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let cookie_canary = site.proving_ground.plant_canary("session_cookie").ok();
    let token_canary = site.proving_ground.plant_canary("endpoint_token").ok();
    let (Some(cookie_canary), Some(token_canary)) = (cookie_canary, token_canary) else {
        checks.check(
            "Proving ground plants session_cookie and endpoint_token canaries",
            false,
        );
        return Ok(());
    };
    let Some(login_url) = site.start_url(COOKIE_TASK).map(str::to_owned) else {
        checks.check("Proving ground serves canaries/3 login task", false);
        return Ok(());
    };
    let Some(account) = cookie_canary.account.clone() else {
        checks.check("canaries/3 provides a test account", false);
        return Ok(());
    };

    let base = target
        .environment
        .get("GOODFETCH_HOME")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("recordings-canary");
    let _ = std::fs::create_dir_all(&base);
    let warc_path = base.join("redacted.warc");

    let mut env_target = target.clone();
    env_target
        .environment
        .insert("GF_REC_CANARY_TOKEN".to_owned(), token_canary.value.clone());
    let mut secret_conn = connection
        .another(&env_target)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let opened = hello_as(&mut secret_conn, Product::Collie, Surface::Rust).is_ok();
    checks.check("engine with Secret canary env var opens", opened);
    if !opened {
        secret_conn.close();
        return Ok(());
    }

    let run_id = "conformance-rec-redact-1";
    let settings = json!({
        "secrets": {
            "placeholders": {
                "api_token": {
                    "reference": "env:GF_REC_CANARY_TOKEN",
                    "domains": [CACHE_HOST],
                }
            }
        }
    });
    let started = start_with_mode(
        &mut secret_conn,
        site,
        run_id,
        &settings,
        "record",
        &warc_path,
    )?;
    checks.check(
        "record Run with Secret placeholder starts",
        started.get("result").is_some(),
    );
    if let Some(page) = new_page(&mut secret_conn, run_id, checks)? {
        send_canary_requests(
            &mut secret_conn,
            site,
            &page,
            &login_url,
            (&account.username, &account.password, &token_canary.value),
        )?;
    }
    let finished = finish_report(&mut secret_conn, run_id)?;
    connection.keep(secret_conn.take_exchanged());
    secret_conn.close();
    checks.check(
        "record Run finishes",
        finished["result"]["lifecycle"] == json!("finished"),
    );

    verify_warc_redacted(
        &base,
        &warc_path,
        &cookie_canary.value,
        &token_canary.value,
        checks,
    );
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous conformance harness reads the recorded WARC file on its own thread"
)]
fn verify_warc_redacted(
    base: &Path,
    warc_path: &Path,
    cookie_value: &str,
    token_value: &str,
    checks: &mut Checks,
) {
    let canaries = [
        ("session_cookie", cookie_value),
        ("endpoint_token", token_value),
    ];
    let scan_dir = base.join("scan-only");
    let _ = std::fs::create_dir_all(&scan_dir);
    let _ = std::fs::copy(warc_path, scan_dir.join("redacted.warc"));
    let leaks = held_under(&scan_dir, &canaries).unwrap_or_default();
    checks.check(
        "Secret canary search finds zero canaries in the recorded WARC",
        leaks.is_empty(),
    );

    let text = String::from_utf8_lossy(&std::fs::read(warc_path).unwrap_or_default()).into_owned();
    let lower = text.to_ascii_lowercase();
    let has_forbidden_req_header = lower.lines().any(|line| {
        line.starts_with("cookie:")
            || line.starts_with("authorization:")
            || line.starts_with("proxy-authorization:")
    });
    let has_unredacted_set_cookie = lower
        .lines()
        .filter(|line| line.starts_with("set-cookie:"))
        .any(|line| !line.contains("[redacted]"));
    checks.check(
        "recorded WARC drops Cookie, Authorization and Proxy-Authorization and masks Set-Cookie values",
        !has_forbidden_req_header && !has_unredacted_set_cookie,
    );
}

fn send_canary_requests(
    conn: &mut Connection,
    site: &Site,
    page: &goodfetch_protocol::page::PageOpened,
    login_url: &str,
    (username, password, token): (&str, &str, &str),
) -> Result<(), Gone> {
    let page_id = json!(page.page_id);
    let _ = conn.goto(page, login_url)?;
    let _ = call(
        conn,
        PAGE_REQUEST.name,
        &page_id,
        &json!({
            "url": login_url,
            "method": "POST",
            "headers": {
                "content-type": "application/x-www-form-urlencoded",
                "authorization": format!("Bearer {token}"),
                "proxy-authorization": "Basic cHJveHk6c2VjcmV0",
            },
            "post_data": data_encoding::BASE64.encode(
                format!("username={username}&password={password}").as_bytes()
            ),
        }),
    )?;
    let api_url = site.named_url(CACHE_HOST, "/api/secret-check");
    let _ = call(
        conn,
        PAGE_REQUEST.name,
        &page_id,
        &json!({
            "url": api_url,
            "method": "POST",
            "headers": {
                "content-type": "application/json",
                "x-api-token": "%api_token%",
                "authorization": "Bearer %api_token%",
            },
            "post_data": data_encoding::BASE64.encode(br#"{"secret":"%api_token%"}"#),
        }),
    )?;
    Ok(())
}

/// Case 7 (`story:6-57`): a cut response is recorded with
/// `WARC-Truncated: disconnect` and replays as `response_truncated`.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous conformance harness reads the recorded WARC file on its own thread"
)]
pub(super) fn record_cut_and_replay_truncated(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let warc_path = write_fixture(target, "recorded-cut.warc", b"");
    let cut_url = site.named_url("fetch-edge.test", "/cut/content-length");

    let record_run = "conformance-rec-cut-write";
    let _ = start_with_mode(
        connection,
        site,
        record_run,
        &json!({}),
        "record",
        &warc_path,
    )?;
    let Some(page) = new_page(connection, record_run, checks)? else {
        return Ok(());
    };
    let page_id = json!(page.page_id);
    let answer = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": cut_url }),
    )?;
    checks.check(
        "live cut request answers 200 head",
        answer["result"]["status"] == json!(200),
    );
    let _ = finish_report(connection, record_run)?;

    let warc_text =
        String::from_utf8_lossy(&std::fs::read(&warc_path).unwrap_or_default()).into_owned();
    checks.check(
        "recorded WARC marks the cut response with `WARC-Truncated: disconnect`",
        warc_text.contains("WARC-Truncated: disconnect"),
    );

    let replay_run = "conformance-rec-cut-replay";
    let _ = start_with_mode(
        connection,
        site,
        replay_run,
        &json!({}),
        "replay",
        &warc_path,
    )?;
    let Some(rpage) = new_page(connection, replay_run, checks)? else {
        return Ok(());
    };
    let rpage_id = json!(rpage.page_id);
    let ranswer = call(
        connection,
        PAGE_REQUEST.name,
        &rpage_id,
        &json!({ "url": cut_url }),
    )?;
    let req_id = ranswer["result"]["request_id"].as_str().unwrap_or_default();
    let rbody = call(
        connection,
        RESPONSE_BODY.name,
        &rpage_id,
        &json!({ "request_id": req_id }),
    )?;
    checks.check(
        "replaying the recorded cut response raises `response_truncated`",
        rbody["error"]["data"]["code"] == json!(RESPONSE_TRUNCATED.name),
    );
    let _ = finish_report(connection, replay_run)?;
    Ok(())
}

/// Case 8 (`story:6-55`): a `record` Run reads no cache entry while storing
/// into the cache, and a `record` Run killed and resumed produces one WARC
/// file holding both halves.
pub(super) fn record_cache_and_kill_resume(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    // Part A: `record` reads no cache entry, but may store into the cache.
    let shared_url = site.named_url(CACHE_HOST, "/fresh/rec-cache-bypass");
    let fresh_url = site.named_url(CACHE_HOST, "/fresh/rec-cache-store");
    let cache_settings = json!({ "cache": { "mode": "replay" } });
    let warc_cache = write_fixture(target, "recorded-cache-interaction.warc", b"");

    // Seed cache with Run 1 (`cache.mode: replay`, no recording).
    let run1 = "conformance-rec-cache-seed";
    let settings1 = site.mapped(&cache_settings, connection.acts_for_model());
    let id1 = connection.request(
        RUN_START.name,
        &json!({ "run_id": run1, "settings": settings1 }),
    )?;
    let _ = connection.answer(id1)?;
    if let Some(p1) = new_page(connection, run1, checks)? {
        let _ = connection.goto(&p1, &shared_url)?;
    }
    let _ = finish_report(connection, run1)?;

    // Run 2 (`cache.mode: replay` + `recording: record`) MUST hit the site for `shared_url`
    // AND store `fresh_url` into the cache.
    let before_record = total_received(site);
    let run2 = "conformance-rec-cache-live";
    let _ = start_with_mode(
        connection,
        site,
        run2,
        &cache_settings,
        "record",
        &warc_cache,
    )?;
    if let Some(p2) = new_page(connection, run2, checks)? {
        let _ = connection.goto(&p2, &shared_url)?;
        let _ = connection.goto(&p2, &fresh_url)?;
    }
    let _ = finish_report(connection, run2)?;
    let after_record = total_received(site);
    checks.check(
        "record Run bypasses cache read and fetches live from Proving ground",
        after_record >= before_record + 2,
    );

    // Run 3 (`cache.mode: replay`, no recording) gets `fresh_url` from the cache
    // without hitting the Proving ground.
    let before_cache_hit = total_received(site);
    let run3 = "conformance-rec-cache-stored";
    let settings3 = site.mapped(&cache_settings, connection.acts_for_model());
    let id3 = connection.request(
        RUN_START.name,
        &json!({ "run_id": run3, "settings": settings3 }),
    )?;
    let _ = connection.answer(id3)?;
    if let Some(p3) = new_page(connection, run3, checks)? {
        let _ = connection.goto(&p3, &fresh_url)?;
    }
    let _ = finish_report(connection, run3)?;
    checks.check(
        "a later cache-replay Run serves what the record Run stored without reaching the site",
        total_received(site) == before_cache_hit,
    );

    verify_kill_and_resume(connection, target, site, checks)
}

fn verify_kill_and_resume(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let warc_resume = write_fixture(target, "recorded-kill-resume.warc", b"");
    let resume_id = "conformance-rec-kill-resume";
    let half1_url = site.named_url(CACHE_HOST, "/fresh/rec-half-1");
    let half2_url = site.named_url(CACHE_HOST, "/fresh/rec-half-2");

    let mut other = connection
        .another(target)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let opened = hello_as(&mut other, Product::Collie, Surface::Rust).is_ok();
    checks.check("second engine opens for kill/resume record test", opened);
    if !opened {
        other.close();
        return Ok(());
    }
    let started1 = start_with_mode(
        &mut other,
        site,
        resume_id,
        &json!({}),
        "record",
        &warc_resume,
    )?;
    checks.check(
        "first half starts on second engine",
        started1["result"]["started_as"] == json!("new"),
    );
    if let Some(p_half1) = new_page(&mut other, resume_id, checks)? {
        let _ = other.goto(&p_half1, &half1_url)?;
    }
    connection.keep(other.take_exchanged());
    checks.check("second engine is killed mid-recording", other.kill());

    // Resume on primary engine with same run_id and recording path.
    let resumed = start_with_mode(
        connection,
        site,
        resume_id,
        &json!({}),
        "record",
        &warc_resume,
    )?;
    checks.check(
        "resumed record Run starts as `resumed`",
        resumed["result"]["started_as"] == json!("resumed"),
    );
    if let Some(p_half2) = new_page(connection, resume_id, checks)? {
        let _ = connection.goto(&p_half2, &half2_url)?;
    }
    let finished = finish_report(connection, resume_id)?;
    checks.check(
        "resumed record Run finishes with both halves counted",
        finished["result"]["lifecycle"] == json!("finished")
            && finished["result"]["recording"]["responses_used"]
                .as_u64()
                .is_some_and(|n| n >= 2),
    );

    // Replay the merged WARC file and verify BOTH halves answer with zero network hits.
    let before_merged = total_received(site);
    let verify_id = "conformance-rec-kill-resume-replay";
    let _ = start_with_mode(
        connection,
        site,
        verify_id,
        &json!({}),
        "replay",
        &warc_resume,
    )?;
    if let Some(p_verify) = new_page(connection, verify_id, checks)? {
        let g1 = connection.goto(&p_verify, &half1_url)?;
        let g2 = connection.goto(&p_verify, &half2_url)?;
        checks.check(
            "replaying merged recording answers both halves without network",
            g1.get("result").is_some()
                && g2.get("result").is_some()
                && total_received(site) == before_merged,
        );
    }
    let _ = finish_report(connection, verify_id)?;
    Ok(())
}

fn eval_text(connection: &mut Connection, page_id: &Value) -> Result<Value, Gone> {
    let reply = call(
        connection,
        PAGE_EVALUATE.name,
        page_id,
        &json!({ "function": "() => document.body ? document.body.innerText.trim() : ''" }),
    )?;
    Ok(reply["result"]["value"].clone())
}
