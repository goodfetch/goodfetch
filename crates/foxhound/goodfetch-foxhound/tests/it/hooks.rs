//! Hooks in code at the request and page stages (ADR 0104), on Foxhound's
//! executable against the Proving ground and Chrome for Testing: on the
//! `browser` tier, each stage's Hooks run in the order the crawl declared
//! them, before request and before navigation before the site sees the
//! request, after response and after navigation once it answered, and page
//! ready just before the handler.

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, outcomes, requests_for, settings};
use crate::frontier::crawl_from;

/// The Proving ground's seed.
const SEED: u64 = 767;

/// A static listing the crawl reaches.
const HOST: &str = "catalog.test";
const PATH: &str = "/catalog/1";

/// A request on a forced `browser` tier goes through a Hook at each of the
/// five request and page stages, two at before request, in the order
/// declared, each seeing the site as its stage says: the request not yet
/// sent before request and before navigation, answered from after
/// response on; then the handler, once.
///
/// Covers: story:3-168
#[test]
fn a_browser_requests_hooks_run_at_each_stage_in_order_around_its_navigation() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let hook = |name: &str, stage: &str| json!({ "name": name, "stage": stage });

    let mut seen = Vec::new();
    crawl_from(
        &mut engine,
        &json!({
            "run_id": "hooks-browser",
            "seeds": [{ "url": proving_ground.url(HOST, PATH), "label": "listing" }],
            "labels": { "listing": {} },
            "settings": settings(
                &proving_ground,
                &json!({
                    "fetch": { "tier": "browser" },
                    "site_preferences": { "respect_robots_txt": false },
                }),
            ),
            "hooks": [
                hook("ready", "page_ready"),
                hook("landed", "after_navigation"),
                hook("look", "after_response"),
                hook("sign", "before_request"),
                hook("steer", "before_navigation"),
                hook("seal", "before_request"),
            ],
        }),
        &mut |_, params| {
            let sent = requests_for(&proving_ground, HOST, PATH).len();
            if params.get("hook").is_some() {
                seen.push((params["stage"].clone(), params["hook"].clone(), sent));
                json!({ "result": {} })
            } else {
                seen.push((json!("handler"), Value::Null, sent));
                json!({ "result": { "records": [] } })
            }
        },
    );
    let ended = outcomes(&mut engine, "hooks-browser");
    assert!(engine.finished());

    assert_eq!(
        (seen, ended),
        (
            vec![
                (json!("before_request"), json!("sign"), 0),
                (json!("before_request"), json!("seal"), 0),
                (json!("before_navigation"), json!("steer"), 0),
                (json!("after_response"), json!("look"), 1),
                (json!("after_navigation"), json!("landed"), 1),
                (json!("page_ready"), json!("ready"), 1),
                (json!("handler"), Value::Null, 1),
            ],
            vec![(
                proving_ground.url(HOST, PATH),
                json!("succeeded"),
                Value::Null
            )],
        )
    );
}
