//! Tier table cases, `fetch.tier_race`'s winner and its counting (spec
//! #582; ADRs 0040, 0086 and 0093; CO-16), on Foxhound's executable
//! against the Proving ground's `tiers.test` and Chrome for Testing: a
//! race starts browser-fetch, then a full navigation in the same Session
//! after its Label on its domain's median browser-fetch latency, 500 ms
//! before any; the first page that passes the Page check wins, the other is
//! cancelled, both reach the site and count in Usage, and Provenance
//! records the race. A handler that escalates browser-fetch's page runs
//! again in the browser, and only that run's staged writes commit, with
//! the attempt (ADR 0093). Judged by what the Proving ground received, the
//! Run's Usage, its run file and its Provenance (ADR 0064): the only time
//! the test sets is the Proving ground's hold, which keeps browser-fetch in
//! flight past the navigation's start.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::fetch_tiers::{engine, expecting, outcomes, provenance, recording, settings, tiers};
use crate::frontier::{Inside, added, crawl_declaring};
use crate::staging::{keys, run_file, work_items};

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

/// How long, in milliseconds, the Proving ground held its first answer to
/// `target` on [`HOST`], from the request's arrival to the answer's send;
/// `None` before it answered one.
fn held_ms(proving_ground: &ProvingGround, target: &str) -> Option<u64> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == HOST && request["target"] == target)
        .find_map(|request| {
            let timing = &request["timing"];
            let answered = timing["answered_microseconds"].as_u64()?;
            Some(answered.saturating_sub(timing["arrived_microseconds"].as_u64()?) / 1_000)
        })
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
/// is cancelled. The site received both, and Usage counts both; the
/// navigation's request reaches the site only as browser-fetch's answer
/// arrives, since Chrome holds a second request for a URL its Session's
/// HTTP cache is already fetching. The case first checks the Proving
/// ground held that answer 3 s, its one premise. The next
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

    // The case's one premise: a Proving ground executable that predates the
    // `hold` query (a stale test tool manifest) answers at once, and every
    // race below would read as browser-fetch winning before the delay.
    let answered_after = held_ms(&proving_ground, "/products/1?hold=3000");
    assert!(
        answered_after.is_some_and(|after| after >= 3_000),
        "the Proving ground held browser-fetch's answer {answered_after:?} ms, not 3000: its executable predates `tiers.test`'s `hold`; build the test tools again (`just test`)"
    );
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

/// One run of the blog's handler: its tier, what its enqueue answered and
/// what its wait for the posts answered (their count, else its error's
/// code).
type BlogRun = (Value, Value, Value);

/// The crawl `run_id` of a racing Run whose warm-up request enqueues
/// `/blog/1` of Label `blog`, so its race starts with the browser running
/// and browser-fetch, given no hold, wins it. The blog's handler enqueues
/// `/products/3?from=<tier>`, sets the key `<tier>` to its attempt through
/// its `ctx.run`, waits for the posts, which escalates its `http` run to
/// the browser, and answers a Record naming its tier, or, in the browser
/// when `fails_in_browser`, fails for good. Answers the crawl's report and
/// the blog's runs.
fn race_escalating(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    fails_in_browser: bool,
) -> (Value, Vec<BlogRun>) {
    let warm = proving_ground.url(HOST, "/products/9");
    let blog = proving_ground.url(HOST, "/blog/1");
    let mut runs = Vec::new();
    let report = crawl_declaring(
        engine,
        run_id,
        &[json!({ "url": warm, "label": "warm" })],
        &json!({ "warm": {}, "blog": {}, "child": {} }),
        &racing(proving_ground),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let label = &handle["crawl_request"]["label"];
            if label == "warm" {
                inside.enqueue(handle, &blog, "blog", None);
            }
            if label != "blog" {
                return json!({ "result": { "records": [] } });
            }
            let tier = handle["tier"].as_str().unwrap().to_owned();
            let enqueued =
                inside.enqueue(handle, &format!("/products/3?from={tier}"), "child", None);
            inside.call(
                "key_value_store.set",
                &json!({
                    "run_id": handle["run_id"],
                    "key": tier,
                    "value": handle["attempt"],
                    "attempt": {
                        "crawl_request_id": handle["crawl_request"]["id"],
                        "lease_epoch": handle["lease_epoch"],
                    },
                }),
            );
            let waited = inside.call(
                "locator.wait_for_count",
                &json!({
                    "locator": { "page_id": handle["page_id"], "selector": "li.item" },
                    "count": { "min": 1 },
                }),
            );
            runs.push((
                json!(tier),
                enqueued,
                waited["result"]["count"].as_u64().map_or_else(
                    || waited["error"]["data"]["code"].clone(),
                    |count| json!(count),
                ),
            ));
            if fails_in_browser && tier == "browser" {
                json!({ "error": { "code": -32003, "message": "my bug", "data": { "code": "handler_failed", "retryable": false } } })
            } else {
                json!({ "result": { "records": [{ "dataset": "pages", "data": { "tier": tier } }] } })
            }
        },
    );
    (report, runs)
}

/// The data of each Record of the dataset `pages` the run file of `run_id`
/// holds, in order.
fn pages(env: &TestEnv, run_id: &str) -> Vec<Value> {
    run_file(env, run_id)
        .prepare("SELECT data FROM records WHERE dataset = 'pages' ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect()
}

/// Each race of the Label `blog` in `run_id`: how browser-fetch ended and
/// the winner.
fn blog_races(env: &TestEnv, run_id: &str) -> Vec<Value> {
    provenance(env, run_id, "tier_race")
        .iter()
        .filter(|entry| entry["label"] == "blog")
        .map(|entry| json!([entry["browser_fetch"], entry["winner"]]))
        .collect()
}

/// A crawl conformance case (ADR 0093): browser-fetch wins the race, and
/// its handler's wait, which the document cannot meet, escalates the
/// attempt to the browser, where the same handler runs again. What the
/// `http` run staged (its enqueue, its key) and its answer's Record are
/// discarded with that answer; the browser run's enqueue, key and Record
/// are committed, each once.
///
/// Covers: story:6-236, story:6-263, setting:fetch.tier_race, operation:crawl_request.enqueue, operation:key_value_store.set, operation:locator.wait_for_count
#[test]
fn an_escalated_race_commits_what_its_browser_run_staged_once_and_nothing_of_its_http_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let (report, runs) = race_escalating(&mut engine, &proving_ground, "escalated", false);
    let ended = outcomes(&mut engine, "escalated");
    assert!(engine.finish());

    let url = |path: &str| proving_ground.url(HOST, path);
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            runs,
            blog_races(&env, "escalated"),
            work_items(&env, "escalated"),
            keys(&env, "escalated"),
            pages(&env, "escalated"),
            ended,
        ),
        (
            &json!("finished"),
            vec![
                (
                    json!("http"),
                    added("3", &url("/products/3?from=http")),
                    json!("attempt_ended")
                ),
                (
                    json!("browser"),
                    added("4", &url("/products/3?from=browser")),
                    json!(3)
                ),
            ],
            vec![json!(["won", "browser_fetch"])],
            vec![
                url("/products/9"),
                url("/blog/1"),
                url("/products/3?from=browser")
            ],
            vec![("browser".to_owned(), "1".to_owned())],
            vec![json!({ "tier": "browser" })],
            vec![
                (url("/blog/1"), json!("succeeded"), Value::Null),
                (
                    url("/products/3?from=browser"),
                    json!("succeeded"),
                    Value::Null
                ),
                (url("/products/9"), json!("succeeded"), Value::Null),
            ],
        ),
    );
}

/// A crawl conformance case (ADR 0093): browser-fetch wins the race and
/// its handler escalates the attempt to the browser, where the handler
/// fails for good: neither run's enqueue, key or Record is committed.
///
/// Covers: story:6-236, story:6-263, setting:fetch.tier_race, operation:crawl_request.enqueue, operation:key_value_store.set, operation:locator.wait_for_count
#[test]
fn an_escalated_race_that_fails_in_the_browser_commits_nothing_either_run_staged() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let (report, runs) = race_escalating(&mut engine, &proving_ground, "failed", true);
    let ended = outcomes(&mut engine, "failed");
    assert!(engine.finish());

    let url = |path: &str| proving_ground.url(HOST, path);
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            runs,
            blog_races(&env, "failed"),
            work_items(&env, "failed"),
            keys(&env, "failed"),
            pages(&env, "failed"),
            ended,
        ),
        (
            &json!("finished"),
            vec![
                (
                    json!("http"),
                    added("3", &url("/products/3?from=http")),
                    json!("attempt_ended")
                ),
                (
                    json!("browser"),
                    added("4", &url("/products/3?from=browser")),
                    json!(3)
                ),
            ],
            vec![json!(["won", "browser_fetch"])],
            vec![url("/products/9"), url("/blog/1")],
            Vec::new(),
            Vec::new(),
            vec![
                (url("/blog/1"), json!("failed"), json!("handler_failed")),
                (url("/products/9"), json!("succeeded"), Value::Null),
            ],
        ),
    );
}
