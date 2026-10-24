//! Politeness conformance cases for the AI-use signals (spec #581's seam:
//! the protocol against the Proving ground; ADRs 0016, 0086 and 0093):
//! crawls under `fetch.tier: http` on Foxhound's executable against the
//! `access-policy` family's TDMRep-header task, whose second page answers
//! with `tdm-reservation: 1`. A user who honours the signals and declares a
//! Purpose the reservation refuses has that page end `skipped` with
//! `use_signals`, its handler never run; a user who does not, or whose
//! Purpose is `search`, has it handled.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::crawl_with;

/// The Proving ground's seed.
const SEED: u64 = 883;

/// The `access-policy` family's TDMRep-header task's host.
const TDMREP_HOST: &str = "5.access-policy.test";

/// Crawls the shelf's first two pages under `site_preferences`; answers
/// the paths the handler was given and each outcome's path, kind and
/// skip reason, by path.
fn crawl(site_preferences: &Value) -> (Vec<String>, Vec<(String, Value, Value)>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip().to_string();
    let settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { TDMREP_HOST: [ip] } },
        "site_preferences": site_preferences,
    });
    let seeds: Vec<Value> = ["/catalog/1", "/catalog/2"]
        .iter()
        .map(|path| json!({ "url": proving_ground.url(TDMREP_HOST, path), "label": "page" }))
        .collect();
    let mut engine = Attached::start(&env);
    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        "signals",
        &seeds,
        &settings,
        &mut |_, handle| {
            handled.push(path_of(&handle["crawl_request"]["url"]));
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "signals" }),
        &mut |_| None,
    );
    assert!(engine.finish());
    let mut outcomes: Vec<(String, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                path_of(&outcome["request"]["url"]),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    handled.sort();
    (handled, outcomes)
}

/// The path of `url`.
fn path_of(url: &Value) -> String {
    let rest = url.as_str().unwrap().split_once("://").unwrap().1;
    rest[rest.find('/').unwrap()..].to_owned()
}

/// With the signals honoured and the Purpose `ai-input`, the page whose
/// response reserves its text and data mining rights ends `skipped` with
/// `use_signals`, and its handler is never run.
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn a_page_whose_tdmrep_header_refuses_the_purpose_ends_skipped_with_use_signals() {
    let (handled, outcomes) = crawl(&json!({ "respect_use_signals": true, "purpose": "ai-input" }));

    assert_eq!(
        (handled, outcomes),
        (
            vec!["/catalog/1".to_owned()],
            vec![
                ("/catalog/1".to_owned(), json!("succeeded"), Value::Null),
                (
                    "/catalog/2".to_owned(),
                    json!("skipped"),
                    json!("use_signals")
                ),
            ],
        )
    );
}

/// The same page is handled while the signals are not honoured, the
/// default, and while the Purpose is `search`, which no signal is judged
/// against (ADR 0086).
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn the_page_is_handled_unless_the_signals_are_honoured_for_a_declared_purpose() {
    let both = vec!["/catalog/1".to_owned(), "/catalog/2".to_owned()];

    assert_eq!(
        (
            crawl(&json!({ "purpose": "ai-input" })).0,
            crawl(&json!({ "respect_use_signals": true })).0,
        ),
        (both.clone(), both)
    );
}
