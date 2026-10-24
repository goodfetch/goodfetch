//! The navigation gate Foxhound sets on a handler's pages (spec #579's
//! `gate`; ADRs 0086, 0087, 0093, 0118 and 0121), on Foxhound's executable
//! against the Proving ground and Chrome for Testing: a page the handler
//! opens in its crawl's Run, and every page opened from it, takes a permit
//! of its domain's Politeness for each navigation, each redirect hop and
//! each `page.request`, and is refused with `site_preferences_refused`
//! where the site's robots.txt, read through the copy the user's runs
//! share, disallows it for `goodfetch-foxhound`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 760;

/// The `access-policy` family's robots.txt host: its `*` group disallows
/// `/private/` and asks for a Crawl-delay of 2 s.
const ROBOTS_HOST: &str = "1.access-policy.test";

/// An engine whose pages launch Chrome for Testing from the test's cache.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    Attached::start_with(env, |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl under `fetch.tier: http` that reaches every
/// host of `proving_ground`, with `more` merged in.
fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let mut settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": resolve },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    settings
}

/// Opens a page in the handler's Run, answering its ID.
fn new_page(inside: &mut Inside<'_>, handle: &Value) -> String {
    let opened = inside.call("run.new_page", &json!({ "run_id": handle["run_id"] }));
    opened["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no page opened: {opened}"))
        .to_owned()
}

/// The Error code, the reason and the URL of a call's refusal.
fn refusal(answer: &Value) -> (Value, Value, Value) {
    let data = &answer["error"]["data"];
    (
        data["code"].clone(),
        data["reason"].clone(),
        data["url"].clone(),
    )
}

/// How many requests for `path` the Proving ground received on `host`.
fn requested(proving_ground: &ProvingGround, host: &str, path: &str) -> usize {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == host && request["target"] == path)
        .count()
}

/// A handler's `goto` to a page the site's robots.txt disallows for
/// `goodfetch-foxhound` raises `site_preferences_refused` with `robots_txt`
/// and never reaches the site, while one it allows loads.
///
/// Covers: story:5-134, error:site_preferences_refused, operation:page.goto
#[test]
fn a_handlers_goto_robots_txt_disallows_is_refused_before_it_leaves() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let disallowed = proving_ground.url(ROBOTS_HOST, "/private/catalog/2");
    let allowed = proving_ground.url(ROBOTS_HOST, "/catalog/3");

    let mut answers = Vec::new();
    let report = crawl_with(
        &mut engine,
        "robots",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &json!({})),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            for url in [&disallowed, &allowed] {
                answers.push(inside.call("page.goto", &json!({ "page_id": page_id, "url": url })));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            refusal(&answers[0]),
            answers[1]["result"]["response"]["status"].clone(),
            requested(&proving_ground, ROBOTS_HOST, "/private/catalog/2"),
        ),
        (
            &json!("finished"),
            (
                json!("site_preferences_refused"),
                json!("robots_txt"),
                json!(disallowed)
            ),
            json!(200),
            0,
        ),
        "{answers:?}"
    );
}
