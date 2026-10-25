//! Tier table cases, `fetch.tier_race`'s winner and its counting (spec
//! #582; ADRs 0040, 0086 and 0093; CO-16), on Foxhound's executable
//! against the Proving ground's `tiers.test` and Chrome for Testing: a
//! race starts browser-fetch, then a full navigation in the same Session
//! after its Label on its domain's median browser-fetch latency, 500 ms
//! before any; the first page that passes the Page check wins, the other is
//! cancelled, both reach the site and count in Usage, and Provenance
//! records the race. Judged by what the Proving ground received, the Run's
//! Usage and its Provenance (ADR 0064): the only time the test sets is the
//! Proving ground's hold, which keeps browser-fetch in flight past the
//! navigation's start.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, expecting, outcomes, provenance, recording, settings, tiers};
use crate::frontier::crawl_declaring;

/// The Proving ground's seed.
const SEED: u64 = 945;

/// The host whose listings are static or script-rendered by path.
const HOST: &str = "tiers.test";

/// The settings of a racing crawl of the Proving ground, two requests at a
/// time on a domain with no delay, so a race's navigation has room beside
/// its browser-fetch (ADR 0086).
fn racing(proving_ground: &ProvingGround) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier_race": true },
            "politeness": { "domain": { "concurrency": 2, "delay": 0 } },
        }),
    )
}

/// How many requests for `target` on [`HOST`] the Proving ground received.
fn received(proving_ground: &ProvingGround, target: &str) -> usize {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == HOST && request["target"] == target)
        .count()
}

/// Each `tier_race` Provenance entry of `run_id`: its Label, its delay,
/// how browser-fetch and the navigation ended, and the winner.
fn races(env: &TestEnv, run_id: &str) -> Vec<Value> {
    provenance(env, run_id, "tier_race")
        .iter()
        .map(|entry| {
            json!([
                entry["label"],
                entry["after_ms"],
                entry["browser_fetch"],
                entry["navigation"],
                entry["winner"]
            ])
        })
        .collect()
}

/// How many of `run_id`'s races sent their navigation.
fn navigations_sent(env: &TestEnv, run_id: &str) -> u64 {
    provenance(env, run_id, "tier_race")
        .iter()
        .filter(|entry| entry["navigation"] != "not_sent")
        .count() as u64
}

/// A static listing held 3 s: browser-fetch's document passes the Page
/// check first, so the handler is given it on `http`, and the navigation
/// started 500 ms after it, the Label having no browser-fetch latency yet,
/// is cancelled. The site received both, and Usage counts both. The next
/// request of the same Label waits its first's latency, the median of one,
/// before its navigation, which never starts: its unheld browser-fetch won
/// before. A first request of another Label warms the browser, so neither
/// run of the held race waits for it to launch; that request's own race
/// may go either way, so only its count is judged.
///
/// Covers: story:6-263, setting:fetch.tier_race, operation:crawl.start
#[test]
fn browser_fetch_that_passes_first_wins_and_its_navigation_is_cancelled_both_counted() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let warm = proving_ground.url(HOST, "/products/9");
    let held = proving_ground.url(HOST, "/products/1?hold=3000");
    let next = proving_ground.url(HOST, "/products/2");

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "won-by-fetch",
        &[
            json!({ "url": warm, "label": "warm" }),
            json!({ "url": held, "label": "list" }),
            json!({ "url": next, "label": "list" }),
        ],
        &json!({ "warm": {}, "list": {} }),
        &racing(&proving_ground),
        &mut recording(&mut seen),
    );
    let ended = outcomes(&mut engine, "won-by-fetch");
    assert!(engine.finish());

    let mut list_races: Vec<Value> = races(&env, "won-by-fetch")
        .into_iter()
        .filter(|race| race[0] == "list")
        .collect();
    let next_delay = list_races[1][1].clone();
    list_races[1][1] = Value::Null;
    let mut chosen: Vec<Value> = tiers(&env, "won-by-fetch")
        .into_iter()
        .filter(|entry| entry[0] == "list")
        .collect();
    chosen.sort_by_key(Value::to_string);
    seen.retain(|(url, _)| *url != warm);
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            ended
                .iter()
                .map(|ended| ended.1.clone())
                .collect::<Vec<_>>(),
            received(&proving_ground, "/products/1?hold=3000"),
            received(&proving_ground, "/products/2"),
            list_races,
            next_delay.as_u64().is_some_and(|delay| delay >= 3_000),
            chosen,
            &report["result"]["usage"]["requests"],
        ),
        (
            &json!("finished"),
            vec![(held, json!("http")), (next, json!("http"))],
            vec![json!("succeeded"); 3],
            2,
            1,
            vec![
                json!(["list", 500, "won", "cancelled", "browser_fetch"]),
                json!(["list", null, "won", "not_sent", "browser_fetch"]),
            ],
            true,
            vec![json!(["list", "http", "race", null, null]); 2],
            // Three crawl requests and each navigation sent.
            &json!(3 + navigations_sent(&env, "won-by-fetch")),
        ),
    );
}

/// A script-rendered listing whose Label expects its entries: the document
/// browser-fetch serves lacks them and fails the Page check, so the
/// navigation started 500 ms after it wins, its page, where the script ran,
/// passing; the handler is given the browser's page only. The site received
/// both, and Usage counts both. A first request of another Label warms the
/// browser, so browser-fetch does not wait for it to launch; that
/// request's own race may go either way, so only its count is judged.
///
/// Covers: story:6-263, setting:fetch.tier_race, setting:session.page_check
#[test]
fn a_navigation_wins_when_browser_fetch_fails_the_page_check() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let warm = proving_ground.url(HOST, "/products/9");
    let blog = proving_ground.url(HOST, "/blog/1");

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "won-by-navigation",
        &[
            json!({ "url": warm, "label": "warm" }),
            json!({ "url": blog, "label": "blog" }),
        ],
        &json!({ "warm": {}, "blog": expecting("li.item", &json!({})) }),
        &racing(&proving_ground),
        &mut recording(&mut seen),
    );
    let ended = outcomes(&mut engine, "won-by-navigation");
    assert!(engine.finish());

    seen.retain(|(url, _)| *url != warm);
    let of_blog = |entries: Vec<Value>| -> Vec<Value> {
        entries
            .into_iter()
            .filter(|entry| entry[0] == "blog")
            .collect()
    };
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            ended,
            received(&proving_ground, "/blog/1"),
            of_blog(races(&env, "won-by-navigation")),
            of_blog(tiers(&env, "won-by-navigation")),
            &report["result"]["usage"]["requests"],
        ),
        (
            &json!("finished"),
            vec![(blog.clone(), json!("browser"))],
            vec![
                (blog, json!("succeeded"), Value::Null),
                (warm, json!("succeeded"), Value::Null),
            ],
            2,
            vec![json!(["blog", 500, "failed", "won", "browser"])],
            vec![json!(["blog", "browser", "race", null, null])],
            // Two crawl requests and each navigation sent.
            &json!(2 + navigations_sent(&env, "won-by-navigation")),
        ),
    );
}
