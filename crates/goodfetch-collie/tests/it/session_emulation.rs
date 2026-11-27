//! Session device emulation and network throttling over stdio (spec #578,
//! ticket #3289; ADRs 0083 and 0092; pitfalls B17 and B18): every
//! `browser.*` emulation setting and `network.throttle` is set where a
//! Session opens or above, never per page (`setting_not_applicable`),
//! refused on a Borrowed browser (`setting_not_applicable`), locked against
//! an imported login (`setting_locked`) while an inherited value yields,
//! and Chromium-only keys raise `capability_unavailable` outside Chromium.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests measure real throttled vs unthrottled navigation durations against the Proving ground"
)]

use std::time::{Duration, Instant};

use goodfetch_protocol::page::session::SessionOpened;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, cli};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::page::{opened, started};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 3289;

fn open_session(engine: &mut goodfetch_test_client::Attached, id: i64, params: &Value) -> Value {
    engine.ask(id, "run.new_session", params)
}

fn eval(engine: &mut goodfetch_test_client::Attached, id: i64, page_id: &str, expr: &str) -> Value {
    let answer = engine.ask(
        id,
        "page.evaluate",
        &json!({ "page_id": page_id, "function": expr }),
    );
    assert!(
        answer.get("error").is_none(),
        "page.evaluate failed: {answer}"
    );
    answer["result"]["value"].clone()
}

fn assert_setting_error(answer: &Value, code: &str, key: &str) {
    assert_eq!(
        (
            answer["error"]["data"]["code"].clone(),
            answer["error"]["data"]["key"].clone(),
        ),
        (json!(code), json!(key)),
        "{answer}"
    );
}

fn open_session_page_at(
    engine: &mut goodfetch_test_client::Attached,
    base_id: i64,
    session_params: &Value,
    url: &str,
) -> String {
    let session_ans = open_session(engine, base_id, session_params);
    let session: SessionOpened = serde_json::from_value(session_ans["result"].clone())
        .unwrap_or_else(|e| panic!("{e}: {session_ans}"));
    let page = opened(&engine.ask(
        base_id + 1,
        "session.new_page",
        &json!({ "session_id": session.session_id }),
    ));
    let went = engine.ask(
        base_id + 2,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    assert!(went.get("error").is_none(), "{went}");
    page.page_id.0
}

/// Settings conformance cases, the `browser.*` lines: setting `browser.*`
/// or `network.throttle` on a page's own call fails with
/// `setting_not_applicable`; setting `browser.*` where a Borrowed browser's
/// Session opens fails with `setting_not_applicable`; an explicit
/// `browser.locale` where an imported-login Session opens fails with
/// `setting_locked` naming `session.source`, while an inherited one yields
/// in `config explain`.
///
/// Covers: story:5-59, setting:browser.viewport, setting:browser.locale, setting:network.throttle, error:setting_not_applicable, error:setting_locked
#[test]
fn browser_settings_enforce_session_scope_borrowed_refusal_and_import_yielding() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();

    // Inherited `browser.locale` yields to an imported login in `config explain`.
    env.user_configuration(
        "[settings.browser]\n\
         locale = \"fr-FR\"\n\
         \n\
         [settings.session.source]\n\
         kind = \"state_file\"\n\
         path = \"login.json\"\n",
    );
    let explained = cli(&env, COLLIE)
        .args(["config", "explain", "browser.locale", "--json"])
        .run()
        .unwrap();
    assert_eq!(explained.exit, Some(0), "{explained}");
    let explained_json: Value = serde_json::from_str(&explained.stdout).unwrap();
    assert_eq!(
        explained_json["settings"][0]["yields_to"]["key"],
        json!("session.source"),
        "{explained_json}"
    );

    // Clear user config for the live engine scope tests.
    let env = TestEnv::new();
    let mut engine = started(&env, &cache, &proving_ground, "scopes");

    // 1. `browser.viewport` on `run.new_page` fails with `setting_not_applicable`.
    let page_viewport = engine.ask(
        2,
        "run.new_page",
        &json!({
            "run_id": "scopes",
            "settings": { "browser": { "viewport": { "width": 375, "height": 812 } } }
        }),
    );
    assert_setting_error(&page_viewport, "setting_not_applicable", "browser.viewport");

    // 2. Open a valid Session, then try `browser.locale` and `network.throttle` on `session.new_page`.
    let session_answer = open_session(&mut engine, 3, &json!({ "run_id": "scopes" }));
    let session: SessionOpened = serde_json::from_value(session_answer["result"].clone()).unwrap();

    let page_locale = engine.ask(
        4,
        "session.new_page",
        &json!({
            "session_id": session.session_id,
            "settings": { "browser": { "locale": "fr-FR" } }
        }),
    );
    assert_setting_error(&page_locale, "setting_not_applicable", "browser.locale");

    let page_throttle = engine.ask(
        5,
        "session.new_page",
        &json!({
            "session_id": session.session_id,
            "settings": { "network": { "throttle": { "latency": 100 } } }
        }),
    );
    assert_setting_error(&page_throttle, "setting_not_applicable", "network.throttle");

    // 3. `browser.viewport` set where a Borrowed browser's Session opens fails with `setting_not_applicable`.
    let borrowed = open_session(
        &mut engine,
        6,
        &json!({
            "run_id": "scopes",
            "source": { "kind": "cdp_url", "url": "http://127.0.0.1:9222" },
            "settings": { "browser": { "viewport": { "width": 800, "height": 600 } } }
        }),
    );
    assert_setting_error(&borrowed, "setting_not_applicable", "browser.viewport");

    // 4. `browser.locale` set where an imported-login Session opens fails with `setting_locked` naming `session.source`.
    let locked = open_session(
        &mut engine,
        7,
        &json!({
            "run_id": "scopes",
            "source": { "kind": "state_file", "path": "login.json" },
            "settings": { "browser": { "locale": "de-DE" } }
        }),
    );
    assert_setting_error(&locked, "setting_locked", "browser.locale");
    assert_eq!(
        locked["error"]["data"]["limit"]["key"],
        json!("session.source"),
        "{locked}"
    );

    let _ = engine.ask(8, "run.finish", &json!({ "run_id": "scopes" }));
    let (_, exited) = engine.finish();
    assert!(exited);
}

fn assert_desktop_default_layout(engine: &mut goodfetch_test_client::Attached, layout_url: &str) {
    let desktop_page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "b18" })));
    let went = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": desktop_page.page_id, "url": layout_url }),
    );
    assert!(went.get("error").is_none(), "{went}");
    let desktop = eval(
        engine,
        4,
        &desktop_page.page_id.0,
        r"() => ({
            width: window.innerWidth,
            height: window.innerHeight,
            dpr: window.devicePixelRatio,
            touch: navigator.maxTouchPoints,
            below768: window.matchMedia('(max-width: 767px)').matches,
            wideDisplay: getComputedStyle(document.querySelector('.plan.wide')).display,
            narrowDisplay: getComputedStyle(document.querySelector('.plan.narrow')).display,
            dark: window.matchMedia('(prefers-color-scheme: dark)').matches,
        })",
    );
    // Full headless Chromium applies `--window-size=1280,720` to outer window
    // bounds (leaving ~143px of browser chrome on Linux so `innerHeight` is
    // 577; `chrome-headless-shell` reports 720). Exact default viewport
    // override waits for #4054 / #2183's command batching so
    // `C7.opening_commands` stays at 13.
    let desktop_height = desktop["height"].as_i64().unwrap_or_default();
    assert!(
        desktop_height > 0 && desktop_height <= 720,
        "unexpected default desktop height: {desktop}"
    );
    assert_eq!(
        (
            desktop["width"].clone(),
            desktop["dpr"].clone(),
            desktop["touch"].clone(),
            desktop["below768"].clone(),
            desktop["wideDisplay"].clone(),
            desktop["narrowDisplay"].clone(),
            desktop["dark"].clone(),
        ),
        (
            json!(1280),
            json!(1),
            json!(0),
            json!(false),
            json!("block"),
            json!("none"),
            json!(false),
        ),
        "desktop observation: {desktop}"
    );
}

fn assert_mobile_emulated_layout(engine: &mut goodfetch_test_client::Attached, layout_url: &str) {
    let page_id = open_session_page_at(
        engine,
        5,
        &json!({
            "run_id": "b18",
            "settings": {
                "browser": {
                    "viewport": { "width": 375, "height": 812 },
                    "has_touch": true,
                    "device_scale_factor": 2,
                    "is_mobile": true,
                    "color_scheme": "dark",
                    "locale": "fr-FR",
                    "timezone_id": "Asia/Tokyo",
                }
            }
        }),
        layout_url,
    );
    let mobile = eval(
        engine,
        8,
        &page_id,
        r"() => ({
            width: window.innerWidth,
            height: window.innerHeight,
            dpr: window.devicePixelRatio,
            hasTouch: navigator.maxTouchPoints >= 1,
            below768: window.matchMedia('(max-width: 767px)').matches,
            wideDisplay: getComputedStyle(document.querySelector('.plan.wide')).display,
            narrowDisplay: getComputedStyle(document.querySelector('.plan.narrow')).display,
            dark: window.matchMedia('(prefers-color-scheme: dark)').matches,
            language: navigator.language,
            intlLocale: Intl.DateTimeFormat().resolvedOptions().locale,
            timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        })",
    );
    assert_eq!(
        (
            mobile["width"].clone(),
            mobile["height"].clone(),
            mobile["dpr"].clone(),
            mobile["hasTouch"].clone(),
            mobile["below768"].clone(),
            mobile["wideDisplay"].clone(),
            mobile["narrowDisplay"].clone(),
            mobile["dark"].clone(),
            mobile["language"].clone(),
            mobile["intlLocale"]
                .as_str()
                .is_some_and(|l| l.starts_with("fr")),
            mobile["timeZone"].clone(),
        ),
        (
            json!(375),
            json!(812),
            json!(2),
            json!(true),
            json!(true),
            json!("none"),
            json!("block"),
            json!(true),
            json!("fr-FR"),
            true,
            json!("Asia/Tokyo"),
        ),
        "mobile observation: {mobile}"
    );
}

/// Pitfall B18 and Session device emulation on real Chromium: under default
/// settings (`1280x720`, `has_touch: false`, `color_scheme: light`), the
/// Proving ground's responsive layout renders desktop (`wide`) markup; under
/// `browser.viewport` 375x812 with `browser.has_touch: true`,
/// `device_scale_factor: 2`, `is_mobile: true`, `color_scheme: dark`,
/// `locale: fr-FR` and `timezone_id: Asia/Tokyo`, it renders mobile
/// (`narrow`) markup with all overrides observed. With
/// `browser.java_script_enabled: false`, page scripts do not execute.
///
/// Covers: story:5-59, setting:browser.viewport, setting:browser.has_touch, setting:browser.device_scale_factor, setting:browser.is_mobile, setting:browser.color_scheme, setting:browser.locale, setting:browser.timezone_id, setting:browser.java_script_enabled
#[test]
fn b18_responsive_layout_reads_mobile_markup_under_375x812_with_touch_and_desktop_under_default() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "b18");
    let layout_url = proving_ground.url("1.responsive.test", "/layout");

    assert_desktop_default_layout(&mut engine, &layout_url);
    assert_mobile_emulated_layout(&mut engine, &layout_url);

    // JS disabled Session: page script (`keys.js`, which sets `dataset.posting = "0"`) does not run.
    let keys_url = proving_ground.url("1.keys.test", "/type");
    let no_js_page_id = open_session_page_at(
        &mut engine,
        9,
        &json!({
            "run_id": "b18",
            "settings": { "browser": { "java_script_enabled": false } }
        }),
        &keys_url,
    );
    let posting = eval(
        &mut engine,
        12,
        &no_js_page_id,
        "() => document.documentElement.dataset.posting ?? null",
    );
    assert_eq!(
        posting,
        Value::Null,
        "page script ran despite java_script_enabled: false"
    );

    let _ = engine.ask(13, "run.finish", &json!({ "run_id": "b18" }));
    let (_, exited) = engine.finish();
    assert!(exited);
}

/// `network.throttle` slows the Proving ground's page on Chromium;
/// `browser.is_mobile` and `network.throttle` outside Chromium raise
/// `capability_unavailable`.
///
/// Covers: story:5-59, setting:network.throttle, setting:browser.is_mobile, error:capability_unavailable
#[test]
fn network_throttle_slows_chromium_and_chromium_only_settings_refuse_other_browsers() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "throttle");
    let url = proving_ground.url("1.responsive.test", "/layout");

    // Non-Chromium capability checks.
    let firefox_mobile = open_session(
        &mut engine,
        2,
        &json!({
            "run_id": "throttle",
            "source": {
                "kind": "profile_directory",
                "path": "/tmp/firefox-profile",
                "browser": "firefox",
                "domains": ["example.com"]
            },
            "settings": { "browser": { "is_mobile": true } }
        }),
    );
    assert_eq!(
        (
            firefox_mobile["error"]["data"]["code"].clone(),
            firefox_mobile["error"]["data"]["key"].clone(),
        ),
        (json!("capability_unavailable"), json!("browser.is_mobile")),
        "{firefox_mobile}"
    );

    let webkit_throttle = open_session(
        &mut engine,
        3,
        &json!({
            "run_id": "throttle",
            "source": {
                "kind": "installed_browser",
                "browser": "safari",
                "domains": ["example.com"]
            },
            "settings": { "network": { "throttle": { "latency": 200 } } }
        }),
    );
    assert_eq!(
        (
            webkit_throttle["error"]["data"]["code"].clone(),
            webkit_throttle["error"]["data"]["key"].clone(),
        ),
        (json!("capability_unavailable"), json!("network.throttle")),
        "{webkit_throttle}"
    );

    // Unthrottled warm baseline on Chromium.
    let fast_page = opened(&engine.ask(4, "run.new_page", &json!({ "run_id": "throttle" })));
    let warm = engine.ask(
        5,
        "page.goto",
        &json!({ "page_id": fast_page.page_id, "url": url }),
    );
    assert!(warm.get("error").is_none(), "{warm}");
    let t0 = Instant::now();
    let fast = engine.ask(
        6,
        "page.goto",
        &json!({ "page_id": fast_page.page_id, "url": url }),
    );
    let fast_elapsed = t0.elapsed();
    assert!(fast.get("error").is_none(), "{fast}");

    // Throttled Session on Chromium: 400 ms added latency slows the page.
    let slow_session_ans = open_session(
        &mut engine,
        7,
        &json!({
            "run_id": "throttle",
            "settings": {
                "network": {
                    "throttle": { "latency": 400 }
                }
            }
        }),
    );
    let slow_session: SessionOpened = serde_json::from_value(slow_session_ans["result"].clone())
        .unwrap_or_else(|e| panic!("{e}: {slow_session_ans}"));
    let slow_page = opened(&engine.ask(
        8,
        "session.new_page",
        &json!({ "session_id": slow_session.session_id }),
    ));
    let t1 = Instant::now();
    let slow = engine.ask(
        9,
        "page.goto",
        &json!({ "page_id": slow_page.page_id, "url": url }),
    );
    let slow_elapsed = t1.elapsed();
    assert!(slow.get("error").is_none(), "{slow}");

    assert!(
        slow_elapsed >= Duration::from_millis(350) && slow_elapsed > fast_elapsed,
        "expected throttled navigation ({slow_elapsed:?}) to be at least 350ms and slower than unthrottled ({fast_elapsed:?})"
    );

    let _ = engine.ask(10, "run.finish", &json!({ "run_id": "throttle" }));
    let (_, exited) = engine.finish();
    assert!(exited);
}
