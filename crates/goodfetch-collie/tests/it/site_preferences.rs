//! Purpose and Site preferences in Provenance, and `politeness.sharing`
//! pressing harder on Collie (ADRs 0016, 0086 and 0088; ticket 643).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, hello, robots_txt_read, robots_txt_readings};
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, cli};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::page::{opened, started_with};

/// Proving ground host whose `/robots.txt` declares `Content-Signal: … ai-train=no`.
const CONTENT_SIGNAL_HOST: &str = "3.access-policy.test";

const SEED: u64 = 643;

/// A run with no Purpose records `search`, a run declaring `ai-train` records
/// `ai-train`, and each page's `robots_txt` Provenance entry carries the
/// Purpose and the Site preferences (`file`, `allowed`, `content_signals`)
/// read from the host's robots.txt (ticket 643).
///
/// Covers: story:4-77, setting:site_preferences.purpose, setting:site_preferences.read_robots_txt_for_navigation
#[test]
fn a_run_records_its_purpose_and_site_preferences_in_provenance() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let url = proving_ground.url(CONTENT_SIGNAL_HOST, "/catalog/1");

    let run_one = |run_id: &str, purpose: Option<&str>| -> (Value, Vec<Value>) {
        let mut site_prefs = Map::new();
        site_prefs.insert("read_robots_txt_for_navigation".to_owned(), json!(true));
        if let Some(purpose) = purpose {
            site_prefs.insert("purpose".to_owned(), json!(purpose));
        }
        let mut settings = Map::new();
        settings.insert("site_preferences".to_owned(), Value::Object(site_prefs));
        let mut engine = started_with(&env, &cache, &proving_ground, run_id, &settings, |_| {});
        let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": run_id })));
        let goto = engine.ask(
            3,
            "page.goto",
            &json!({ "page_id": page.page_id.0, "url": url }),
        );
        assert!(goto.get("result").is_some(), "{goto}");
        robots_txt_read(&env, run_id);
        let report = engine.ask(4, "run.finish", &json!({ "run_id": run_id }));
        let _ = engine.finish();
        (report["result"].clone(), robots_txt_readings(&env, run_id))
    };

    let (default_report, default_readings) = run_one("default-purpose", None);
    let (train_report, train_readings) = run_one("train-purpose", Some("ai-train"));

    let summarize = |report: &Value, readings: &[Value]| {
        let first = readings.first().cloned().unwrap_or(Value::Null);
        let purpose = report["settings"]
            .as_array()
            .and_then(|settings| {
                settings
                    .iter()
                    .find(|setting| setting["key"] == json!("site_preferences.purpose"))
            })
            .map_or(Value::Null, |setting| setting["value"].clone());
        (
            purpose,
            first["purpose"].clone(),
            first["file"].clone(),
            first["allowed"].clone(),
            first["content_signals"]["ai-train"].clone(),
        )
    };
    assert_eq!(
        (
            summarize(&default_report, &default_readings),
            summarize(&train_report, &train_readings),
        ),
        (
            (
                json!("search"),
                json!("search"),
                json!("found"),
                json!(true),
                json!(false),
            ),
            (
                json!("ai-train"),
                json!("ai-train"),
                json!("found"),
                json!(true),
                json!(false),
            ),
        ),
        "{default_readings:?} {train_readings:?}"
    );
}

/// `site_preferences.purpose` rejects any value outside its catalog choices
/// with `setting_invalid` (ticket 643).
///
/// Covers: story:4-77, setting:site_preferences.purpose, error:setting_invalid
#[test]
fn site_preferences_purpose_rejects_an_unknown_value_with_setting_invalid() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let refused = engine.ask(
        1,
        "run.start",
        &json!({
            "run_id": "bad-purpose",
            "settings": { "site_preferences": { "purpose": "scraping" } },
        }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
        ),
        (
            &json!("setting_invalid"),
            &json!("site_preferences.purpose")
        ),
        "{refused}"
    );
}

/// `config explain site_preferences` shows the layer that set
/// `site_preferences.purpose` (ticket 643).
///
/// Covers: story:4-77, command:config.explain, setting:site_preferences.purpose
#[test]
fn config_explain_site_preferences_shows_the_layer_that_set_purpose() {
    let env = TestEnv::new();
    env.user_configuration("site_preferences.purpose = \"ai-train\"\n");

    let printed = cli(&env, COLLIE)
        .args(["config", "explain", "site_preferences", "--json"])
        .run()
        .unwrap();

    let explained: Value = serde_json::from_str(&printed.stdout).unwrap();
    let purpose = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["key"] == json!("site_preferences.purpose"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        (&purpose["value"], &purpose["layer"]),
        (&json!("ai-train"), &json!("user_configuration")),
        "{explained}"
    );
}

/// A Collie Run whose `politeness.sharing` is other than `shared` writes one
/// `politeness_weakened` Audit log entry, a `weakened` item in its Run report,
/// and one warning; `shared` writes none (positive control; ticket 643).
///
/// Covers: story:4-79, setting:politeness.sharing
#[rstest]
#[case::per_daemon("per_daemon")]
#[case::run("run")]
fn a_collie_run_with_politeness_sharing_other_than_shared_records_weakened_audit_and_warning(
    #[case] sharing: &str,
) {
    let run_with = |env: &TestEnv, run_id: &str, value: &str| -> (Value, Vec<Value>, usize) {
        let mut engine = Attached::start(env, COLLIE);
        engine.send(&hello(Product::Collie));
        let _ = engine.next();
        let started = engine.ask(
            1,
            "run.start",
            &json!({
                "run_id": run_id,
                "settings": { "politeness": { "sharing": value } },
            }),
        );
        assert!(started.get("result").is_some(), "{started}");
        let report = engine.ask(2, "run.finish", &json!({ "run_id": run_id }));
        let (_, stderr) = engine.finish_with_stderr();
        let warnings = stderr
            .lines()
            .filter(|line| line.contains("politeness.sharing"))
            .count();
        let exported = cli(env, COLLIE)
            .args(["audit", "export"])
            .run()
            .unwrap()
            .stdout;
        let audit: Vec<Value> = exported
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|entry| entry["kind"] == json!("politeness_weakened"))
            .map(|entry| {
                json!([
                    entry["kind"],
                    entry["details"]["setting"],
                    entry["details"]["value"],
                    entry["details"]["default"],
                ])
            })
            .collect();
        (report["result"]["weakened"].clone(), audit, warnings)
    };

    let (pressed_env, shared_env) = (TestEnv::new(), TestEnv::new());
    let pressed = run_with(&pressed_env, "pressed", sharing);
    let shared = run_with(&shared_env, "shared", "shared");

    assert_eq!(
        (pressed, shared),
        (
            (
                json!([{
                    "key": "politeness.sharing",
                    "value": sharing,
                    "layer": "run",
                    "origin": "settings",
                }]),
                vec![json!([
                    "politeness_weakened",
                    "politeness.sharing",
                    sharing,
                    "shared",
                ])],
                1,
            ),
            (Value::Null, Vec::new(), 0),
        )
    );
}
