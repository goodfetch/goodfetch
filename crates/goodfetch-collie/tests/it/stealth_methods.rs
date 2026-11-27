//! End-to-end tests of `stealth` Fingerprint methods (`none`, `js_injection`,
//! `basic`), `stealth.allow_runtime`, Run report warnings (`warn_once` for
//! `fingerprint_none`), Provenance, Audit `fingerprint_method_chosen` entries,
//! and browser process sharing across methods (ADR 0035, 0075; spec #599,
//! #913).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "reads Run trace and Audit export files once the engine stopped"
)]

use std::fs;
use std::path::Path;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, cli};
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::page::{browser_processes, opened, started_with};
use crate::page_events::Exchange;
use crate::sessions::{page_in, session_of};

const SEED: u64 = 913;

/// Reads entries of `provenance.jsonl` for `run_id`.
fn provenance_entries(home: &Path, run_id: &str) -> Vec<Value> {
    let path = HomeLayout::of(home).run_trace(run_id, RUN_TRACE_PROVENANCE);
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

/// Exports the Audit log of `env`'s Goodfetch home as JSONL entries.
fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let checked = cli(env, COLLIE).args(["audit", "export"]).run().unwrap();
    assert_eq!(
        checked.exit,
        Some(0),
        "audit export failed: {}",
        checked.stderr
    );
    checked
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

/// Warning codes from `run.finish`.
fn warning_codes(finished: &Value) -> Vec<String> {
    finished["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|warning| warning["code"].as_str().map(str::to_owned))
        .collect()
}

/// Builds `stealth` Run settings with headless Chromium and offline egress,
/// merging `stealth` fields.
fn stealth_run(stealth: Value) -> Map<String, Value> {
    Map::from_iter([
        (
            "browser".to_owned(),
            json!({ "access_mode": "stealth", "headless": true }),
        ),
        ("stealth".to_owned(), stealth),
        ("egress".to_owned(), json!({ "offline": true })),
    ])
}

/// Asserts that `page.on("console")` and `page.on("page_error")` are refused
/// while `stealth.allow_runtime` is off.
fn assert_runtime_refused(env: &TestEnv, cache: &Path, proving_ground: &ProvingGround) {
    let settings = stealth_run(json!({ "human_input": false }));
    let mut engine = started_with(env, cache, proving_ground, "rt-off", &settings, |_| {});
    let session = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "rt-off" })));
    let page = page_in(&mut engine, 3, &session).page_id;
    let refused = engine.ask(
        4,
        "page.on",
        &json!({ "page_id": page, "event": "console" }),
    );
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("capability_unavailable"),
        "{refused}"
    );
    assert_eq!(
        refused["error"]["data"]["fix"],
        json!("`stealth.allow_runtime: true`"),
        "{refused}"
    );
    let refused_error = engine.ask(
        5,
        "page.on",
        &json!({ "page_id": page, "event": "page_error" }),
    );
    assert_eq!(
        refused_error["error"]["data"]["code"],
        json!("capability_unavailable"),
        "{refused_error}"
    );
    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "rt-off" }));
    let _ = engine.finish();
}

/// In a `stealth` Session, `page.on("console")` is refused with
/// `capability_unavailable` naming `stealth.allow_runtime: true` while the
/// setting is off, and succeeds once `stealth.allow_runtime: true` is set,
/// emitting `stealth_runtime_allowed` and recording `runtime_allowed: true`
/// in Provenance and the Audit log (spec #599 Q2, #913).
///
/// Covers: setting:stealth.allow_runtime
#[test]
fn stealth_allow_runtime_gates_console_and_records_warning_and_audit() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();

    assert_runtime_refused(&env, &cache, &proving_ground);

    let allowed_settings = stealth_run(json!({ "human_input": false, "allow_runtime": true }));
    let engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "rt-on",
        &allowed_settings,
        |_| {},
    );
    let mut exchange = Exchange::new(engine);
    let session = session_of(&exchange.call("run.new_session", &json!({ "run_id": "rt-on" })));
    let page =
        opened(&exchange.call("session.new_page", &json!({ "session_id": session }))).page_id;
    let url = proving_ground.url("access.test", "/page-events");
    let loaded = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    assert!(loaded.get("result").is_some(), "{loaded}");
    let _ = exchange.listen(&page.0, "console");
    let reloaded = exchange.call("page.reload", &json!({ "page_id": page }));
    assert!(reloaded.get("result").is_some(), "{reloaded}");
    exchange.until("page.console", |message| {
        message["text"] == json!("page-events error")
    });
    let finished = exchange.call("run.finish", &json!({ "run_id": "rt-on" }));
    let _ = exchange.engine.finish();

    let warnings = warning_codes(&finished);
    assert!(
        warnings
            .iter()
            .any(|code| code == "stealth_runtime_allowed"),
        "expected stealth_runtime_allowed warning, got {warnings:?}"
    );
    let prov = provenance_entries(env.home(), "rt-on");
    assert!(
        prov.iter().any(|entry| {
            entry["action"] == json!("fingerprint_method")
                && entry["runtime_allowed"] == json!(true)
        }),
        "expected Provenance runtime_allowed: true, got {prov:?}"
    );
    let audits = audit_entries(&env);
    assert!(
        audits.iter().any(|entry| {
            entry["kind"] == json!("fingerprint_method_chosen")
                && entry["details"]["session_id"] == json!(session)
                && entry["details"]["runtime_allowed"] == json!(true)
        }),
        "expected Audit fingerprint_method_chosen with runtime_allowed=true, got {audits:?}"
    );
}

/// Checks that `js_injection` masks `navigator.userAgent`, `platform`,
/// `languages` and `screen` on `page`.
fn assert_js_injection_masks(
    engine: &mut goodfetch_test_client::Attached,
    id: i64,
    page: &goodfetch_protocol::page::PageId,
) {
    let observed = engine.ask(
        id,
        "page.evaluate",
        &json!({
            "page_id": page,
            "function": "() => ({ ua: navigator.userAgent, platform: navigator.platform, languages: navigator.languages, width: screen.width, availWidth: screen.availWidth })",
        }),
    );
    assert!(observed.get("result").is_some(), "{observed}");
    let value = &observed["result"]["value"];
    assert!(
        !value["ua"].as_str().unwrap().contains("HeadlessChrome"),
        "js_injection leaked HeadlessChrome: {value}"
    );
    assert!(
        !value["platform"].as_str().unwrap().is_empty(),
        "js_injection platform empty: {value}"
    );
    assert!(
        !value["languages"].as_array().unwrap().is_empty(),
        "js_injection languages empty: {value}"
    );
    assert!(
        value["width"].as_u64().unwrap() > 0 && value["availWidth"].as_u64().unwrap() > 0,
        "js_injection screen missing: {value}"
    );
}

/// Verifies that the multi-method Run emitted `fingerprint_none` once,
/// warned for `js_injection` and `basic`, recorded all four methods plus
/// `main_world_injection` in Provenance, and wrote four Audit entries.
fn assert_methods_reported(env: &TestEnv, finished: &Value) {
    let warnings = warning_codes(finished);
    let none_warnings = warnings
        .iter()
        .filter(|code| code.as_str() == "fingerprint_none")
        .count();
    assert_eq!(
        none_warnings, 1,
        "fingerprint_none must appear once per Run report: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|code| code == "fingerprint_js_injection"),
        "expected fingerprint_js_injection warning: {warnings:?}"
    );
    assert!(
        warnings.iter().any(|code| code == "fingerprint_basic"),
        "expected fingerprint_basic warning: {warnings:?}"
    );

    let prov = provenance_entries(env.home(), "methods");
    let methods_in_prov: Vec<&str> = prov
        .iter()
        .filter(|entry| entry["action"] == json!("fingerprint_method"))
        .filter_map(|entry| entry["method"].as_str())
        .collect();
    assert_eq!(methods_in_prov.len(), 4, "{prov:?}");
    assert!(methods_in_prov.contains(&"none"));
    assert!(methods_in_prov.contains(&"js_injection"));
    assert!(methods_in_prov.contains(&"basic"));
    assert!(
        prov.iter()
            .any(|entry| entry["action"] == json!("main_world_injection")),
        "expected main_world_injection Provenance mark for js_injection: {prov:?}"
    );

    let audits = audit_entries(env);
    let chosen: Vec<&Value> = audits
        .iter()
        .filter(|entry| entry["kind"] == json!("fingerprint_method_chosen"))
        .collect();
    assert_eq!(chosen.len(), 4, "{chosen:?}");
}

/// Across `none`, `js_injection` and `basic` Sessions in one Run:
/// - two `none` Sessions emit `fingerprint_none` only once in the Run report,
/// - `js_injection` masks `navigator.userAgent`, `platform`, `languages` and
///   `screen` on the Proving ground's `/identity` probe page, emits
///   `fingerprint_js_injection`, and records main-world injection in Provenance,
/// - every Session writes its `fingerprint_method` Provenance mark and
///   `fingerprint_method_chosen` Audit entry, and
/// - all three methods share a single Chromium browser process (ADR 0035, #913).
///
/// Covers: story:8-12, story:8-14, story:8-94, story:8-107
#[test]
fn stealth_methods_warn_once_mask_js_injection_and_share_one_browser_process() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();

    let base_settings = stealth_run(json!({ "fingerprint_method": "none", "human_input": false }));
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "methods",
        &base_settings,
        |_| {},
    );

    let none_1 = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "methods" })));
    let none_2 = session_of(&engine.ask(3, "run.new_session", &json!({ "run_id": "methods" })));
    let js_session = session_of(&engine.ask(
        4,
        "run.new_session",
        &json!({
            "run_id": "methods",
            "settings": { "stealth": { "fingerprint_method": "js_injection" } }
        }),
    ));
    let basic_session = session_of(&engine.ask(
        5,
        "run.new_session",
        &json!({
            "run_id": "methods",
            "settings": { "stealth": { "fingerprint_method": "basic" } }
        }),
    ));

    let url = proving_ground.url("access.test", "/identity");
    for (idx, sess) in [&none_1, &none_2, &js_session, &basic_session]
        .into_iter()
        .enumerate()
    {
        let base_id = 10 + i64::try_from(idx).unwrap() * 4;
        let page = page_in(&mut engine, base_id, sess).page_id;
        let loaded = engine.ask(
            base_id + 1,
            "page.goto",
            &json!({ "page_id": page, "url": url }),
        );
        assert!(loaded.get("result").is_some(), "{loaded}");
        if sess == &js_session {
            assert_js_injection_masks(&mut engine, base_id + 2, &page);
        }
    }

    let browsers: Vec<(u32, String)> = browser_processes(&env)
        .into_iter()
        .filter(|(_, line)| !line.contains("--type="))
        .collect();
    assert_eq!(
        browsers.len(),
        1,
        "none, js_injection and basic Sessions in one Run must share one browser process: {browsers:?}"
    );

    let finished = engine.ask(40, "run.finish", &json!({ "run_id": "methods" }));
    let _ = engine.finish();
    assert_methods_reported(&env, &finished);
}

/// A page whose response carries a Block cause never enters the Response
/// cache: under `cache.mode: "replay"`, a normal page is served from the
/// cache on the second Run without reaching the site, whereas a blocked
/// page (`/corpus/captcha`, 200 OK with a captcha widget) still reaches the
/// site and raises `blocked` again (ADR 0046, #871).
///
/// Covers: story:8-65
#[test]
fn blocked_page_never_enters_response_cache_and_replay_reaches_site() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let replay_settings = Map::from_iter([
        ("cache".to_owned(), json!({ "mode": "replay" })),
        ("egress".to_owned(), json!({ "offline": true })),
    ]);
    let ok_url = proving_ground.url("access.test", "/identity");
    let blocked_url = proving_ground.url("access.test", "/corpus/captcha");

    for run_id in ["replay-1", "replay-2"] {
        let mut engine = started_with(
            &env,
            &cache,
            &proving_ground,
            run_id,
            &replay_settings,
            |_| {},
        );
        let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": run_id }))).page_id;
        let ok = engine.ask(3, "page.goto", &json!({ "page_id": page, "url": ok_url }));
        assert_eq!(
            ok["result"]["response"]["status"],
            json!(200),
            "{run_id}: {ok}"
        );
        let blocked = engine.ask(
            4,
            "page.goto",
            &json!({ "page_id": page, "url": blocked_url }),
        );
        assert_eq!(
            blocked["error"]["data"]["code"],
            json!("blocked"),
            "{run_id}: {blocked}"
        );
        assert_eq!(
            blocked["error"]["data"]["cause"],
            json!("captcha"),
            "{run_id}: {blocked}"
        );
        let _ = engine.ask(5, "run.finish", &json!({ "run_id": run_id }));
        let _ = engine.finish();
    }

    let requests = proving_ground.received_requests().unwrap();
    let ok_hits = requests
        .of_the_case()
        .filter(|request| request.target == "/identity")
        .count();
    let blocked_hits = requests
        .of_the_case()
        .filter(|request| request.target == "/corpus/captcha")
        .count();
    assert_eq!(
        (ok_hits, blocked_hits),
        (1, 2),
        "normal page should replay from cache once; blocked page must reach site on both Runs"
    );
}
