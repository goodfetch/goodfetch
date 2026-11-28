//! SDK-only cookies (`session.cookies`, `session.add_cookies`,
//! `session.clear_cookies`) and per-page Web Storage (`page.local_storage`,
//! `page.session_storage`) bounded by a source's `domains` (ADR 0092;
//! ticket #746).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests start a local Chrome for domain-bound cookie checks"
)]
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end cookie and Web Storage verification exercises navigation, page.request, Proving ground records, and domain bounds together"
)]

use std::path::Path;
use std::process::Stdio;

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, Hello};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, Process, ProvingGround, TestEnv, browser_cache, chrome_headless,
    reported_debugging_port,
};
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::page::{BROWSER_DEADLINE, opened, started_with};

const SEED: u64 = 746;
const SITE: &str = "site.test";
const OTHER: &str = "other.test";

/// Proving ground `scope.test` requests matching `prefix`, returning `(target, cookies)`.
fn scope_requests(
    proving_ground: &ProvingGround,
    prefix: &str,
) -> Vec<(String, Map<String, Value>)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == "scope.test" && request.target.starts_with(prefix))
        .map(|request| {
            let cookies = request
                .cookies
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            (request.target.clone(), cookies)
        })
        .collect()
}

/// Covers: story:5-77, operation:session.cookies, operation:session.add_cookies, operation:session.clear_cookies, operation:page.local_storage, operation:page.session_storage
#[test]
fn cookies_and_web_storage_round_trip_and_affect_navigation_and_page_request() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "cookies",
        &Map::new(),
        |_| {},
    );

    let session_id =
        engine.ask(1, "run.new_session", &json!({ "run_id": "cookies" }))["result"]["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
    let page = opened(&engine.ask(2, "session.new_page", &json!({ "session_id": session_id })));

    // Navigate to Proving ground's /storage endpoint, which sets a session cookie and localStorage.
    let initial_url = proving_ground.url("scope.test", "/storage?mark=alpha");
    let nav = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": initial_url }),
    );
    assert!(nav["error"].is_null(), "initial goto failed: {nav}");

    // Add both a persistent cookie (with RFC 3339 expiry within Chromium's
    // 400-day cap) and a session cookie (no expiry).
    let expiry = "2027-06-15T12:30:00Z";
    let added = engine.ask(
        4,
        "session.add_cookies",
        &json!({
            "session_id": session_id,
            "cookies": [
                {
                    "name": "persistent_tok",
                    "value": "p123",
                    "url": proving_ground.url("scope.test", "/"),
                    "expires": expiry,
                    "http_only": true,
                    "same_site": "strict",
                },
                {
                    "name": "session_tok",
                    "value": "s456",
                    "domain": "scope.test",
                    "path": "/",
                    "same_site": "lax",
                },
            ]
        }),
    );
    assert!(added["error"].is_null(), "add_cookies failed: {added}");

    // Read cookies back and verify Playwright's fields, RFC 3339 expiry, and `expires: null`.
    let read = engine.ask(
        5,
        "session.cookies",
        &json!({
            "session_id": session_id,
            "urls": [proving_ground.url("scope.test", "/storage")],
        }),
    );
    assert!(read["error"].is_null(), "session.cookies failed: {read}");
    let cookies = read["result"]["cookies"].as_array().unwrap();
    let persistent = cookies
        .iter()
        .find(|c| c["name"] == "persistent_tok")
        .expect("persistent_tok cookie must be returned");
    assert_eq!(persistent["value"], "p123");
    assert_eq!(persistent["domain"], "scope.test");
    assert_eq!(persistent["path"], "/");
    assert_eq!(persistent["http_only"], true);
    assert_eq!(persistent["same_site"], "strict");
    assert_eq!(persistent["expires"], expiry);
    assert!(persistent["secure"].is_boolean());

    let session_cookie = cookies
        .iter()
        .find(|c| c["name"] == "session_tok")
        .expect("session_tok cookie must be returned");
    assert_eq!(session_cookie["value"], "s456");
    assert_eq!(session_cookie["domain"], "scope.test");
    assert_eq!(session_cookie["path"], "/");
    assert_eq!(session_cookie["http_only"], false);
    assert_eq!(session_cookie["same_site"], "lax");
    assert!(
        session_cookie
            .as_object()
            .unwrap()
            .get("expires")
            .is_some_and(Value::is_null),
        "session cookie must serialize `expires: null`: {session_cookie}"
    );

    // Next navigation AND page.request must both send the added cookies.
    let nav2 = engine.ask(
        6,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage?mark=after_add"),
        }),
    );
    assert!(nav2["error"].is_null(), "second goto failed: {nav2}");
    let req1 = engine.ask(
        7,
        "page.request",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage/found?phase=after_add"),
        }),
    );
    assert!(req1["error"].is_null(), "page.request 1 failed: {req1}");

    let nav_after_add = scope_requests(&proving_ground, "/storage?mark=after_add");
    assert_eq!(nav_after_add.len(), 1);
    assert_eq!(
        nav_after_add[0].1.get("persistent_tok"),
        Some(&json!("p123"))
    );
    assert_eq!(nav_after_add[0].1.get("session_tok"), Some(&json!("s456")));

    let req_after_add = scope_requests(&proving_ground, "/storage/found?phase=after_add");
    assert_eq!(req_after_add.len(), 1);
    assert_eq!(
        req_after_add[0].1.get("persistent_tok"),
        Some(&json!("p123"))
    );
    assert_eq!(req_after_add[0].1.get("session_tok"), Some(&json!("s456")));

    // Clear only `session_tok` by name, verify `persistent_tok` stays and `session_tok` is gone.
    let cleared_one = engine.ask(
        8,
        "session.clear_cookies",
        &json!({ "session_id": session_id, "name": "session_tok" }),
    );
    assert!(
        cleared_one["error"].is_null(),
        "clear_cookies(name) failed: {cleared_one}"
    );

    let req2 = engine.ask(
        9,
        "page.request",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage/found?phase=after_partial_clear"),
        }),
    );
    assert!(req2["error"].is_null(), "page.request 2 failed: {req2}");
    let req_partial = scope_requests(&proving_ground, "/storage/found?phase=after_partial_clear");
    assert_eq!(req_partial.len(), 1);
    assert_eq!(req_partial[0].1.get("persistent_tok"), Some(&json!("p123")));
    assert!(!req_partial[0].1.contains_key("session_tok"));

    // Clear all cookies and verify neither navigation nor page.request sends them.
    let cleared_all = engine.ask(
        10,
        "session.clear_cookies",
        &json!({ "session_id": session_id }),
    );
    assert!(
        cleared_all["error"].is_null(),
        "clear_cookies(all) failed: {cleared_all}"
    );

    let nav3 = engine.ask(
        11,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage?mark=after_clear"),
        }),
    );
    assert!(nav3["error"].is_null(), "third goto failed: {nav3}");
    let req3 = engine.ask(
        12,
        "page.request",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage/found?phase=after_clear"),
        }),
    );
    assert!(req3["error"].is_null(), "page.request 3 failed: {req3}");

    let nav_after_clear = scope_requests(&proving_ground, "/storage?mark=after_clear");
    assert_eq!(nav_after_clear.len(), 1);
    assert!(!nav_after_clear[0].1.contains_key("persistent_tok"));
    assert!(!nav_after_clear[0].1.contains_key("session_tok"));

    let req_after_clear = scope_requests(&proving_ground, "/storage/found?phase=after_clear");
    assert_eq!(req_after_clear.len(), 1);
    assert!(!req_after_clear[0].1.contains_key("persistent_tok"));
    assert!(!req_after_clear[0].1.contains_key("session_tok"));

    // Verify per-page localStorage and sessionStorage operations.
    let ls_set = engine.ask(
        13,
        "page.local_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "set_item", "key": "theme", "value": "dark" },
        }),
    );
    assert_eq!(ls_set["result"], json!({ "kind": "updated" }));
    let ls_get = engine.ask(
        14,
        "page.local_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "get_item", "key": "theme" },
        }),
    );
    assert_eq!(ls_get["result"], json!({ "kind": "item", "value": "dark" }));

    let ss_set = engine.ask(
        15,
        "page.session_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "set_item", "key": "tab_token", "value": "page1" },
        }),
    );
    assert_eq!(ss_set["result"], json!({ "kind": "updated" }));
    let ss_items = engine.ask(
        16,
        "page.session_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "items" },
        }),
    );
    assert_eq!(
        ss_items["result"],
        json!({ "kind": "items", "items": { "tab_token": "page1" } })
    );

    // A second page in the same Session on the same origin shares localStorage
    // but has its own isolated sessionStorage.
    let page2 = opened(&engine.ask(17, "session.new_page", &json!({ "session_id": session_id })));
    let nav_p2 = engine.ask(
        18,
        "page.goto",
        &json!({
            "page_id": page2.page_id,
            "url": proving_ground.url("scope.test", "/storage?mark=p2"),
        }),
    );
    assert!(nav_p2["error"].is_null(), "page2 goto failed: {nav_p2}");

    let page2_local = engine.ask(
        19,
        "page.local_storage",
        &json!({
            "page_id": page2.page_id,
            "action": { "kind": "get_item", "key": "theme" },
        }),
    );
    assert_eq!(
        page2_local["result"],
        json!({ "kind": "item", "value": "dark" })
    );

    let page2_session = engine.ask(
        20,
        "page.session_storage",
        &json!({
            "page_id": page2.page_id,
            "action": { "kind": "get_item", "key": "tab_token" },
        }),
    );
    assert_eq!(
        page2_session["result"],
        json!({ "kind": "item", "value": null })
    );

    // remove_item and clear work as expected.
    let ls_rm = engine.ask(
        21,
        "page.local_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "remove_item", "key": "theme" },
        }),
    );
    assert_eq!(ls_rm["result"], json!({ "kind": "updated" }));
    let ls_after_rm = engine.ask(
        22,
        "page.local_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "get_item", "key": "theme" },
        }),
    );
    assert_eq!(
        ls_after_rm["result"],
        json!({ "kind": "item", "value": null })
    );

    let ss_clear = engine.ask(
        23,
        "page.session_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "clear" },
        }),
    );
    assert_eq!(ss_clear["result"], json!({ "kind": "updated" }));
    let ss_after_clear = engine.ask(
        24,
        "page.session_storage",
        &json!({
            "page_id": page.page_id,
            "action": { "kind": "items" },
        }),
    );
    assert_eq!(
        ss_after_clear["result"],
        json!({ "kind": "items", "items": {} })
    );
}

struct RunningChrome {
    process: Process,
    cdp_url: String,
}

impl RunningChrome {
    fn start(env: &TestEnv, cache: &Path, name: &str) -> Self {
        let profile = env.folder().join(name);
        let mut command = env.process_command(chrome_headless(cache));
        command
            .args(chromium_background_traffic_off())
            .args([
                "--headless",
                "--no-first-run",
                "--use-mock-keychain",
                "--password-store=basic",
                "--remote-debugging-port=0",
                "--host-resolver-rules=MAP * 127.0.0.1",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stderr(Stdio::null());
        let process = env.spawn(&mut command).unwrap();
        let port = reported_debugging_port(&profile);
        let file = std::fs::read_to_string(profile.join("DevToolsActivePort")).unwrap();
        let ws_path = file.lines().nth(1).unwrap().trim().to_owned();
        let cdp_url = format!("ws://127.0.0.1:{port}{ws_path}");
        Self { process, cdp_url }
    }

    fn stop(self) {
        let _ = self.process.interrupt();
        let _ = self.process.child().wait();
    }
}

/// H20: On a Session bound to its source's `domains`, a cookie outside those
/// domains is neither read nor written (`domain_not_allowed`), while cookies
/// and storage inside the bound domains work normally.
/// Covers: story:5-77
#[test]
fn domain_bound_session_refuses_cookies_outside_source_domains() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let chrome = RunningChrome::start(&env, &cache, "bound-cookies-chrome");

    let mut engine = Attached::start_with(&env, COLLIE, BROWSER_DEADLINE, |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    engine.send(
        &Hello::needing(Product::Collie)
            .surface(Surface::SdkTypescript)
            .message(),
    );
    let _ = engine.next();
    let started = engine.ask(
        1,
        "run.start",
        &json!({
            "run_id": "bound-cookies",
            "settings": {
                "egress": {
                    "private_ranges": ["127.0.0.0/8"],
                },
            }
        }),
    );
    assert!(started["error"].is_null(), "run.start failed: {started}");

    // First open an unbound session on the same borrowed Chrome to seed a cookie on OTHER.
    let unbound_session = engine.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "bound-cookies",
            "source": {
                "kind": "cdp_url",
                "url": chrome.cdp_url,
            }
        }),
    )["result"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let seeded = engine.ask(
        3,
        "session.add_cookies",
        &json!({
            "session_id": unbound_session,
            "cookies": [{
                "name": "other_secret",
                "value": "must_not_leak",
                "domain": OTHER,
                "path": "/",
            }]
        }),
    );
    assert!(
        seeded["error"].is_null(),
        "seeding other.test cookie failed: {seeded}"
    );

    // Now open a Session bound strictly to `domains: [SITE]`.
    let bound_session = engine.ask(
        4,
        "run.new_session",
        &json!({
            "run_id": "bound-cookies",
            "source": {
                "kind": "cdp_url",
                "url": chrome.cdp_url,
                "domains": [SITE],
            }
        }),
    )["result"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Writing a cookie outside `domains` (via `domain` or via `url`) fails with `domain_not_allowed`.
    let refused_domain = engine.ask(
        5,
        "session.add_cookies",
        &json!({
            "session_id": bound_session,
            "cookies": [{
                "name": "bad",
                "value": "1",
                "domain": OTHER,
                "path": "/",
            }]
        }),
    );
    assert_eq!(
        refused_domain["error"]["data"]["code"],
        "domain_not_allowed"
    );
    assert_eq!(refused_domain["error"]["data"]["domain"], OTHER);

    let refused_url = engine.ask(
        6,
        "session.add_cookies",
        &json!({
            "session_id": bound_session,
            "cookies": [{
                "name": "bad_url",
                "value": "1",
                "url": format!("http://{OTHER}/"),
            }]
        }),
    );
    assert_eq!(refused_url["error"]["data"]["code"], "domain_not_allowed");

    // Reading cookies for an unlisted URL fails with `domain_not_allowed`.
    let refused_read = engine.ask(
        7,
        "session.cookies",
        &json!({
            "session_id": bound_session,
            "urls": [format!("http://{OTHER}/")],
        }),
    );
    assert_eq!(refused_read["error"]["data"]["code"], "domain_not_allowed");

    // Clearing cookies for an unlisted domain fails with `domain_not_allowed`.
    let refused_clear = engine.ask(
        8,
        "session.clear_cookies",
        &json!({
            "session_id": bound_session,
            "domain": OTHER,
        }),
    );
    assert_eq!(refused_clear["error"]["data"]["code"], "domain_not_allowed");

    // Adding a cookie inside `domains` succeeds, and unfiltered `session.cookies`
    // returns only `SITE` cookies (never leaking `other_secret`).
    let added_allowed = engine.ask(
        9,
        "session.add_cookies",
        &json!({
            "session_id": bound_session,
            "cookies": [{
                "name": "allowed_tok",
                "value": "ok",
                "domain": SITE,
                "path": "/",
            }]
        }),
    );
    assert!(
        added_allowed["error"].is_null(),
        "allowed add_cookies failed: {added_allowed}"
    );

    let listed = engine.ask(
        10,
        "session.cookies",
        &json!({ "session_id": bound_session }),
    );
    assert!(
        listed["error"].is_null(),
        "bound session.cookies failed: {listed}"
    );
    let names: Vec<&str> = listed["result"]["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(
        names.contains(&"allowed_tok"),
        "expected allowed_tok in {names:?}"
    );
    assert!(
        !names.contains(&"other_secret"),
        "cookie outside bound domains leaked: {names:?}"
    );

    // Clearing cookies on the bound session clears `SITE` without disturbing `OTHER`.
    let cleared = engine.ask(
        11,
        "session.clear_cookies",
        &json!({ "session_id": bound_session }),
    );
    assert!(
        cleared["error"].is_null(),
        "bound clear_cookies failed: {cleared}"
    );

    let still_in_unbound = engine.ask(
        12,
        "session.cookies",
        &json!({ "session_id": unbound_session, "urls": [format!("http://{OTHER}/")] }),
    );
    assert!(
        still_in_unbound["result"]["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "other_secret"),
        "clearing a bound session must not delete cookies outside its domains: {still_in_unbound}"
    );
    let _ = engine.ask(13, "run.finish", &json!({ "run_id": "bound-cookies" }));
    let _ = engine.finish();
    chrome.stop();
}
