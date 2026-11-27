//! Session permissions and the clipboard's refusal over stdio (spec #578,
//! ticket #3290; ADR 0083):
//! - `browser.permissions` (Playwright's names) grants permissions to every
//!   page of the Session;
//! - `browser.permissions: ["geolocation"]` with `browser.geolocation` is
//!   observed by every page of the Session;
//! - clipboard permissions (`clipboard-read`, `clipboard-write`) are refused
//!   with `setting_not_applicable` on a headful Session (`browser.headless:
//!   false`) while allowed on a headless Session;
//! - a page copies and pastes through keyboard shortcuts and `evaluate` under
//!   the granted permission, and no clipboard operation exists in the catalog.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::page::session::SessionOpened;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::page::{opened, started, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 3290;

fn secure(proving_ground: &ProvingGround, host: &str, path: &str) -> String {
    format!("https://{host}:{}{path}", proving_ground.address().port())
}

fn trust_proving_ground(env: &TestEnv, proving_ground: &ProvingGround, hosts: &[&str]) {
    let ca = proving_ground.description().ca.display().to_string();
    let mut toml = String::from("[settings.tls.trusted_cas]\n");
    for host in hosts {
        toml.push_str(&format!("\"{host}\" = [{}]\n", json!(ca)));
    }
    env.user_configuration(&toml);
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

/// `browser.permissions: ["geolocation"]` with `browser.geolocation` is
/// observed by every page of the Session.
///
/// Covers: story:5-39, setting:browser.permissions, setting:browser.geolocation
#[test]
fn geolocation_permission_and_override_are_observed_by_every_page_of_the_session() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    trust_proving_ground(&env, &proving_ground, &["1.responsive.test"]);
    let mut engine = started(&env, &cache, &proving_ground, "geo");
    let url = secure(&proving_ground, "1.responsive.test", "/layout");

    let session_ans = engine.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "geo",
            "settings": {
                "browser": {
                    "permissions": ["geolocation"],
                    "geolocation": {
                        "latitude": 48.8566,
                        "longitude": 2.3522,
                        "accuracy": 12.5,
                    }
                }
            }
        }),
    );
    assert!(session_ans.get("error").is_none(), "{session_ans}");
    let session: SessionOpened = serde_json::from_value(session_ans["result"].clone()).unwrap();

    // Open two pages in the same Session; both must observe the granted permission and coordinates.
    let read_position = r#"() => new Promise((resolve, reject) => {
        navigator.geolocation.getCurrentPosition(
            (pos) => resolve({
                latitude: pos.coords.latitude,
                longitude: pos.coords.longitude,
                accuracy: pos.coords.accuracy,
            }),
            (err) => reject(new Error(`${err.code}:${err.message}`)),
            { timeout: 10000 }
        );
    })"#;

    let mut next_id = 3;
    for page_index in 0..2 {
        let page = opened(&engine.ask(
            next_id,
            "session.new_page",
            &json!({ "session_id": session.session_id }),
        ));
        next_id += 1;
        let went = engine.ask(
            next_id,
            "page.goto",
            &json!({ "page_id": page.page_id, "url": url }),
        );
        next_id += 1;
        assert!(
            went.get("error").is_none(),
            "page {page_index} goto failed: {went}"
        );
        let coords = eval(&mut engine, next_id, &page.page_id.0, read_position);
        next_id += 1;
        assert_eq!(
            coords,
            json!({
                "latitude": 48.8566,
                "longitude": 2.3522,
                "accuracy": 12.5,
            }),
            "page {page_index} geolocation: {coords}"
        );
    }

    let _ = engine.ask(next_id, "run.finish", &json!({ "run_id": "geo" }));
    let (_, exited) = engine.finish();
    assert!(exited);
}

/// A headful Session (`browser.headless: false`) with a clipboard permission
/// raises `setting_not_applicable`, while a headless Session accepts it;
/// `browser.permissions` is also refused per page and on a Borrowed browser.
///
/// Covers: story:5-39, story:5-87, setting:browser.permissions, error:setting_not_applicable
#[test]
fn headful_session_refuses_clipboard_permissions_while_headless_allows_them() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();

    // Headful Run (`browser.headless: false`): clipboard-read and clipboard-write are refused.
    let mut headful_settings = Map::new();
    headful_settings.insert("browser".to_owned(), json!({ "headless": false }));
    let mut headful = started_with(
        &env,
        &cache,
        &proving_ground,
        "headful-clip",
        &headful_settings,
        |_| {},
    );
    for (index, permission) in ["clipboard-read", "clipboard-write"].iter().enumerate() {
        let refused = headful.ask(
            i64::try_from(index + 2).unwrap(),
            "run.new_session",
            &json!({
                "run_id": "headful-clip",
                "settings": { "browser": { "permissions": [permission] } }
            }),
        );
        assert_eq!(
            (
                refused["error"]["data"]["code"].clone(),
                refused["error"]["data"]["key"].clone(),
            ),
            (
                json!("setting_not_applicable"),
                json!("browser.permissions")
            ),
            "expected headful refusal for {permission}: {refused}"
        );
    }
    let _ = headful.ask(10, "run.finish", &json!({ "run_id": "headful-clip" }));
    let (_, exited) = headful.finish();
    assert!(exited);

    // Headless Run: clipboard permissions succeed; per-page and Borrowed browser are refused.
    let mut headless = started(&env, &cache, &proving_ground, "headless-clip");
    let opened_session = headless.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "headless-clip",
            "settings": {
                "browser": { "permissions": ["clipboard-read", "clipboard-write"] }
            }
        }),
    );
    assert!(opened_session.get("error").is_none(), "{opened_session}");
    let session: SessionOpened = serde_json::from_value(opened_session["result"].clone()).unwrap();

    let per_page = headless.ask(
        3,
        "session.new_page",
        &json!({
            "session_id": session.session_id,
            "settings": { "browser": { "permissions": ["geolocation"] } }
        }),
    );
    assert_eq!(
        (
            per_page["error"]["data"]["code"].clone(),
            per_page["error"]["data"]["key"].clone(),
        ),
        (
            json!("setting_not_applicable"),
            json!("browser.permissions")
        ),
        "{per_page}"
    );

    let borrowed = headless.ask(
        4,
        "run.new_session",
        &json!({
            "run_id": "headless-clip",
            "source": { "kind": "cdp_url", "url": "http://127.0.0.1:9222" },
            "settings": { "browser": { "permissions": ["geolocation"] } }
        }),
    );
    assert_eq!(
        (
            borrowed["error"]["data"]["code"].clone(),
            borrowed["error"]["data"]["key"].clone(),
        ),
        (
            json!("setting_not_applicable"),
            json!("browser.permissions")
        ),
        "{borrowed}"
    );

    let _ = headless.ask(5, "run.finish", &json!({ "run_id": "headless-clip" }));
    let (_, exited) = headless.finish();
    assert!(exited);
}

/// A page copies and pastes through keyboard shortcuts and `evaluate` under
/// the granted clipboard permissions, and no clipboard operation exists in
/// the catalog.
///
/// Covers: story:5-87, setting:browser.permissions
#[test]
fn page_copies_and_pastes_via_keyboard_shortcuts_and_evaluate_and_no_clipboard_operation_exists() {
    // No clipboard operation exists in the protocol catalog.
    let clipboard_ops: Vec<&str> = MODULES
        .iter()
        .flat_map(|module| module.operations.iter())
        .map(|op| op.name)
        .filter(|name| name.contains("clipboard"))
        .collect();
    assert!(
        clipboard_ops.is_empty(),
        "unexpected clipboard operations in catalog: {clipboard_ops:?}"
    );

    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    trust_proving_ground(&env, &proving_ground, &["1.keys.test"]);
    let mut engine = started(&env, &cache, &proving_ground, "clipboard");
    let url = secure(&proving_ground, "1.keys.test", "/type");

    let session_ans = engine.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "clipboard",
            "settings": {
                "browser": {
                    "permissions": ["clipboard-read", "clipboard-write"]
                }
            }
        }),
    );
    assert!(session_ans.get("error").is_none(), "{session_ans}");
    let session: SessionOpened = serde_json::from_value(session_ans["result"].clone()).unwrap();
    let page = opened(&engine.ask(
        3,
        "session.new_page",
        &json!({ "session_id": session.session_id }),
    ));
    let went = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    assert!(went.get("error").is_none(), "{went}");

    // 1. Fill `#field`, select all and copy via keyboard shortcuts, then read via `evaluate`.
    let filled = engine.ask(
        5,
        "locator.fill",
        &json!({
            "locator": { "page_id": page.page_id, "selector": "#field" },
            "value": "copied-via-shortcut",
        }),
    );
    assert!(filled.get("error").is_none(), "{filled}");

    let select_all = engine.ask(
        6,
        "locator.press",
        &json!({
            "locator": { "page_id": page.page_id, "selector": "#field" },
            "key": "ControlOrMeta+a",
        }),
    );
    assert!(select_all.get("error").is_none(), "{select_all}");

    let copy = engine.ask(
        7,
        "locator.press",
        &json!({
            "locator": { "page_id": page.page_id, "selector": "#field" },
            "key": "ControlOrMeta+c",
        }),
    );
    assert!(copy.get("error").is_none(), "{copy}");

    let read_back = eval(
        &mut engine,
        8,
        &page.page_id.0,
        "async () => await navigator.clipboard.readText()",
    );
    assert_eq!(read_back, json!("copied-via-shortcut"), "{read_back}");

    // 2. Write a new value through `evaluate` (`navigator.clipboard.writeText`), then select all and paste via shortcut.
    let _ = eval(
        &mut engine,
        9,
        &page.page_id.0,
        "async () => { await navigator.clipboard.writeText('pasted-via-shortcut'); return true; }",
    );

    let select_all_again = engine.ask(
        10,
        "locator.press",
        &json!({
            "locator": { "page_id": page.page_id, "selector": "#field" },
            "key": "ControlOrMeta+a",
        }),
    );
    assert!(
        select_all_again.get("error").is_none(),
        "{select_all_again}"
    );

    let paste = engine.ask(
        11,
        "locator.press",
        &json!({
            "locator": { "page_id": page.page_id, "selector": "#field" },
            "key": "ControlOrMeta+v",
        }),
    );
    assert!(paste.get("error").is_none(), "{paste}");

    let field_value = eval(
        &mut engine,
        12,
        &page.page_id.0,
        "() => document.getElementById('field').value",
    );
    assert_eq!(field_value, json!("pasted-via-shortcut"), "{field_value}");

    let _ = engine.ask(13, "run.finish", &json!({ "run_id": "clipboard" }));
    let (_, exited) = engine.finish();
    assert!(exited);
}
