//! Tier table cases, `fetch.tier_race`'s winner and its counting (spec
//! #582; ADRs 0040, 0086 and 0093; CO-16), on Foxhound's executable
//! against the Proving ground's `tiers.test` and Chrome for Testing: a
//! race starts browser-fetch, then a full navigation in the same Session
//! after its Label on its domain's median browser-fetch latency, 500 ms
//! before any; the first page that passes the Page check wins, the other is
//! cancelled, both reach the site and count in Usage, and Provenance
//! records the race. A race does not feed the sample comparison, so a
//! Label's first request on a domain takes its one sample
//! (`fetch.sample.count: 1`) instead, and the requests its handler enqueues
//! race on what it learned. A handler that escalates browser-fetch's page
//! runs again in the browser, and only that run's staged writes commit,
//! with the attempt (ADR 0093). Judged by what the Proving ground received,
//! the Run's Usage, its run file and its Provenance (ADR 0064): the only
//! time the test sets is the Proving ground's hold, which keeps
//! browser-fetch in flight past the navigation's start.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod scripted;

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, expecting, outcomes, provenance, settings, tiers};
use crate::frontier::{Inside, added, crawl_declaring};
use crate::run_file;
use crate::staging::{keys, work_items};

/// The Proving ground's seed.
const SEED: u64 = 945;

/// The host whose listings are static or script-rendered by path.
const HOST: &str = "tiers.test";

/// The settings of a racing crawl of the Proving ground, two requests at a
/// time on a domain with no delay, so a race's navigation has room beside
/// its browser-fetch (ADR 0086), and one sample per Label on a domain.
fn racing(proving_ground: &ProvingGround) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier_race": true, "sample": { "count": 1 } },
            "politeness": { "domain": { "concurrency": 2, "delay": 0 } },
        }),
    )
}

/// The static listing a Label first learns its tier on, by its sample,
/// before its handler enqueues the requests that race.
const LEARN: &str = "/products/8";

/// How many requests for `target` on [`HOST`] the Proving ground received.
fn received(proving_ground: &ProvingGround, target: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == HOST && request.target == target)
        .count()
}

/// How long, in milliseconds, the Proving ground held its first answer to
/// `target` on [`HOST`], from the request's arrival to the answer's send;
/// `None` before it answered one.
fn held_ms(proving_ground: &ProvingGround, target: &str) -> Option<u64> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == HOST && request.target == target)
        .find_map(|request| {
            let timing = &request.timing;
            let answered = timing.answered_microseconds?;
            Some(answered.saturating_sub(timing.arrived_microseconds) / 1_000)
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
/// ground held that answer 3 s, its one premise. The Label learned `http`
/// first, on its sample of another static listing, whose handler enqueued
/// the held one; the held one's enqueues the next request of the Label,
/// which waits its first race's latency, the median of one, before its
/// navigation, which never starts: its unheld browser-fetch won before.
/// The winner's `fetch_tier` entry names it `browser_fetch`, as its race
/// does.
///
/// Covers: story:6-263, setting:fetch.tier_race, operation:crawl.start
#[test]
fn browser_fetch_that_passes_first_wins_and_its_navigation_is_cancelled_both_counted() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let learn = proving_ground.url(HOST, LEARN);
    let held = proving_ground.url(HOST, "/products/1?hold=3000");
    let next = proving_ground.url(HOST, "/products/2");

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "won-by-fetch",
        &[json!({ "url": learn, "label": "list" })],
        &json!({ "list": {} }),
        &racing(&proving_ground),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
            if url == learn {
                inside.enqueue(handle, &held, "list", None);
            } else if url == held {
                inside.enqueue(handle, &next, "list", None);
            }
            seen.push((url, handle["tier"].clone()));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = outcomes(&mut engine, "won-by-fetch");
    assert!(engine.finished());

    // The case's one premise: a Proving ground executable that predates the
    // `hold` query (a stale test tool manifest) answers at once, and every
    // race below would read as browser-fetch winning before the delay.
    let answered_after = held_ms(&proving_ground, "/products/1?hold=3000");
    assert!(
        answered_after.is_some_and(|after| after >= 3_000),
        "the Proving ground held browser-fetch's answer {answered_after:?} ms, not 3000: its executable predates `tiers.test`'s `hold`; build the test tools again (`just test`)"
    );
    let mut list_races = races(&env, "won-by-fetch");
    let next_delay = list_races[1][1].clone();
    list_races[1][1] = Value::Null;
    let mut chosen = tiers(&env, "won-by-fetch");
    chosen.sort_by_key(Value::to_string);
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
            vec![
                (learn.clone(), json!("http")),
                (learn, json!("browser")),
                (held, json!("http")),
                (next, json!("http")),
            ],
            vec![json!("succeeded"); 3],
            2,
            1,
            vec![
                json!(["list", 500, "won", "cancelled", "browser_fetch"]),
                json!(["list", null, "won", "not_sent", "browser_fetch"]),
            ],
            true,
            vec![
                json!(["list", "browser", "sample", null, null]),
                json!(["list", "browser_fetch", "race", null, null]),
                json!(["list", "browser_fetch", "race", null, null]),
                json!(["list", "browser_fetch", "sample", null, null]),
            ],
            // The sample's `http` run, the two races' browser-fetches and
            // each navigation sent: a browser run counts none.
            &json!(3 + navigations_sent(&env, "won-by-fetch")),
        ),
    );
}

/// A script-rendered listing whose Label expects its entries: the Label's
/// sample of another of its listings fails the Page check on `http`, so the
/// Label learns `browser`, and that sample's browser run enqueues the
/// listing that races. The document browser-fetch serves lacks the entries
/// and fails the Page check, so the navigation started 500 ms after it
/// wins, its page, where the script ran, passing; the handler is given the
/// browser's page only. The site received both, and Usage counts both.
///
/// Covers: story:6-263, setting:fetch.tier_race, setting:session.page_check
#[test]
fn a_navigation_wins_when_browser_fetch_fails_the_page_check() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let learn = proving_ground.url(HOST, "/blog/2");
    let blog = proving_ground.url(HOST, "/blog/1");

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "won-by-navigation",
        &[json!({ "url": learn, "label": "blog" })],
        &json!({ "blog": expecting("li.item", &json!({})) }),
        &racing(&proving_ground),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
            if url == learn {
                inside.enqueue(handle, &blog, "blog", None);
            }
            seen.push((url, handle["tier"].clone()));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = outcomes(&mut engine, "won-by-navigation");
    assert!(engine.finished());

    let raced = |entries: Vec<Value>| -> Vec<Value> {
        entries
            .into_iter()
            .filter(|entry| entry[2] == "race")
            .collect()
    };
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            ended,
            received(&proving_ground, "/blog/1"),
            races(&env, "won-by-navigation"),
            raced(tiers(&env, "won-by-navigation")),
            &report["result"]["usage"]["requests"],
        ),
        (
            &json!("finished"),
            vec![
                (learn.clone(), json!("browser")),
                (blog.clone(), json!("browser"))
            ],
            vec![
                (blog, json!("succeeded"), Value::Null),
                (learn, json!("succeeded"), Value::Null),
            ],
            2,
            vec![json!(["blog", 500, "failed", "won", "browser"])],
            vec![json!(["blog", "browser", "race", null, null])],
            // The sample's `http` run, the race's browser-fetch and each
            // navigation sent: a browser run counts none.
            &json!(2 + navigations_sent(&env, "won-by-navigation")),
        ),
    );
}

/// One run of the blog's handler: its tier, what its enqueue answered and
/// what its wait for the posts answered (their count, else its error's
/// code).
type BlogRun = (Value, Value, Value);

/// The crawl `run_id` of a racing Run whose first request, a static
/// listing of Label `blog`, is its sample, which learns `http` and
/// enqueues `/blog/1` of the same Label, so its race starts with the
/// browser running and agreeing with the Label, and browser-fetch, given
/// no hold, wins it. The blog's handler enqueues
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
    let learn = proving_ground.url(HOST, LEARN);
    let blog = proving_ground.url(HOST, "/blog/1");
    let mut runs = Vec::new();
    let report = crawl_declaring(
        engine,
        run_id,
        &[json!({ "url": learn, "label": "blog" })],
        &json!({ "blog": {}, "child": {} }),
        &racing(proving_ground),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["url"] == learn.as_str() {
                inside.enqueue(handle, &blog, "blog", None);
                return json!({ "result": { "records": [] } });
            }
            if handle["crawl_request"]["label"] != "blog" {
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
    assert!(engine.finished());

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
                    added("4", &url("/products/3?from=http")),
                    json!("attempt_ended")
                ),
                (
                    json!("browser"),
                    added("5", &url("/products/3?from=browser")),
                    json!(3)
                ),
            ],
            vec![json!(["won", "browser_fetch"])],
            vec![url(LEARN), url("/blog/1"), url("/products/3?from=browser")],
            vec![("browser".to_owned(), "1".to_owned())],
            vec![json!({ "tier": "browser" })],
            vec![
                (url("/blog/1"), json!("succeeded"), Value::Null),
                (
                    url("/products/3?from=browser"),
                    json!("succeeded"),
                    Value::Null
                ),
                (url(LEARN), json!("succeeded"), Value::Null),
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
    assert!(engine.finished());

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
                    added("4", &url("/products/3?from=http")),
                    json!("attempt_ended")
                ),
                (
                    json!("browser"),
                    added("5", &url("/products/3?from=browser")),
                    json!(3)
                ),
            ],
            vec![json!(["won", "browser_fetch"])],
            vec![url(LEARN), url("/blog/1")],
            Vec::new(),
            Vec::new(),
            vec![
                (url("/blog/1"), json!("failed"), json!("handler_failed")),
                (url(LEARN), json!("succeeded"), Value::Null),
            ],
        ),
    );
}

/// An attempt abandoned while its race's browser-fetch page is with its
/// handler, as `run.abort` abandons it, closes that page (issue 2510): a
/// page left open would hold its Run's handles past the Run, and the
/// engine would wait for them once its input ends. The Label learns
/// `http` on its sample of a static listing, whose handler enqueues
/// another, held 3 s; browser-fetch wins that one's race, as the first
/// case's does, and its handler never answers: the Client aborts the Run
/// instead, then ends its input.
///
/// Covers: story:6-263, setting:fetch.tier_race, operation:run.abort
#[test]
fn an_attempt_aborted_while_its_races_browser_fetch_page_is_handled_leaves_no_page_open() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let learn = proving_ground.url(HOST, LEARN);
    let raced = proving_ground.url(HOST, "/products/1?hold=3000");
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": {
            "run_id": "aborted",
            "seeds": [{ "url": learn, "label": "list" }],
            "labels": { "list": {} },
            "settings": racing(&proving_ground),
        },
    }));

    let raced_on = loop {
        let message = engine
            .next()
            .expect("the engine closed before the race's handler ran");
        if message["method"] != "crawl_request.handle" {
            continue;
        }
        let handle = &message["params"];
        if handle["crawl_request"]["url"] == raced.as_str() {
            // The race's handler holds its page from here, and never
            // answers.
            break handle["tier"].clone();
        }
        // Each run of the sample enqueues the request that races, and
        // answers once the engine staged it: the two runs then stage the
        // same, so the Label learns `http` and browser-fetch may win.
        let _ = engine.call(
            "enqueue",
            "crawl_request.enqueue",
            &json!({
                "run_id": handle["run_id"],
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
                "url": raced,
                "label": "list",
            }),
            &mut |_| None,
        );
        engine.send(&json!({
            "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
        }));
    };
    let aborted = engine.call(
        "abort",
        "run.abort",
        &json!({ "run_id": "aborted" }),
        &mut |_| None,
    );
    let exited = engine.finished();

    assert_eq!(
        (raced_on, &aborted["result"]["lifecycle"], exited),
        (json!("http"), &json!("stopped"), true)
    );
}
