//! The navigation gate Foxhound sets on a handler's pages (spec #579's
//! `gate`; ADRs 0086, 0087, 0093, 0118 and 0121), on Foxhound's executable
//! against the Proving ground and Chrome for Testing: a page the handler
//! opens in its crawl's Run, and every page opened from it, takes a permit
//! of its domain's Politeness for each navigation, each redirect hop and
//! each `page.request`, and is refused with `site_preferences_refused`
//! where the site's robots.txt, read through the copy the user's runs
//! share, disallows it for `goodfetch-foxhound`.
#![expect(
    clippy::disallowed_methods,
    reason = "the handler, the test's own code, waits in real time for the popup the browser navigates"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, Instant};

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

/// The navigation test whose page links to `/b` with `#next`.
const HISTORY: &str = "2.navigation.test";

/// The fetch edge pages: redirects and a script's redirect.
const FETCH_EDGE: &str = "fetch-edge.test";

/// The access family's host, whose popup page opens a popup with `#open`.
const POPUP_HOST: &str = "1.access.test";

/// The delay between starts the gate's cases set: long enough that two
/// requests of one domain sent without it would arrive closer.
const DELAY_MILLISECONDS: u64 = 1_000;

/// Each request `host` received for one of `targets`, its target and
/// arrival time in microseconds, in arrival order.
fn arrivals(proving_ground: &ProvingGround, host: &str, targets: &[&str]) -> Vec<(String, u64)> {
    let mut arrivals: Vec<(String, u64)> = proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == host)
        .filter_map(|request| {
            let target = request["target"].as_str()?;
            targets.contains(&target).then(|| {
                (
                    target.to_owned(),
                    request["timing"]["arrived_microseconds"].as_u64().unwrap(),
                )
            })
        })
        .collect();
    arrivals.sort_by_key(|(_, at)| *at);
    arrivals
}

/// The targets of `arrivals` that came less than the delay after the one
/// before them.
fn too_soon(arrivals: &[(String, u64)]) -> Vec<String> {
    // The Proving ground stamps an arrival a little after the engine sent it.
    let least = DELAY_MILLISECONDS * 1_000 * 9 / 10;
    arrivals
        .windows(2)
        .filter(|pair| pair[1].1 - pair[0].1 < least)
        .map(|pair| pair[1].0.clone())
        .collect()
}

/// A URL matcher for a page's waits: the glob `pattern`.
fn glob(pattern: &str) -> Value {
    json!({ "kind": "glob", "pattern": pattern })
}

/// A handler's `goto`, a click that navigates, a `page.request`, each
/// redirect hop, a script's redirect and a popup's navigation each take a
/// permit of their domain's Politeness: under `politeness.domain.delay`,
/// every one of them, and the robots.txt read for their host, reaches the
/// site at least the delay after the one before it there.
///
/// Covers: story:5-134, story:5-112, story:5-113, setting:politeness.domain.delay, operation:page.goto, operation:locator.click, operation:page.request, operation:page.wait_for_url
#[test]
fn every_navigation_of_a_handlers_pages_takes_a_permit_of_its_domain() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let more = json!({ "politeness": { "domain": { "delay": DELAY_MILLISECONDS } } });

    let mut failed = Vec::new();
    let report = crawl_with(
        &mut engine,
        "permits",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            let url = |host: &str, path: &str| json!({ "url": proving_ground.url(host, path) });
            let click =
                |selector: &str| json!({ "locator": { "page_id": page_id, "selector": selector } });
            let steps = [
                ("page.goto", url(HISTORY, "/")),
                ("locator.click", click("#next")),
                ("page.wait_for_url", json!({ "url": glob("**/b") })),
                ("page.request", url(HISTORY, "/c")),
                ("page.goto", url(FETCH_EDGE, "/redirect/hops/2")),
                ("page.goto", url(FETCH_EDGE, "/refresh/script")),
                ("page.wait_for_url", json!({ "url": glob("**/target") })),
                ("page.goto", url(POPUP_HOST, "/popup")),
                ("locator.click", click("#open")),
            ];
            for (method, mut params) in steps {
                if method != "locator.click" {
                    params["page_id"] = json!(page_id);
                }
                let answer = inside.call(method, &params);
                if answer.get("result").is_none() {
                    failed.push(answer);
                }
            }
            // The popup navigates on its own, once its permit comes: the
            // handler answers once it did, while its lease still holds.
            let until = Instant::now() + Duration::from_secs(10);
            while requested(&proving_ground, POPUP_HOST, "/popup/opened") == 0
                && Instant::now() < until
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());

    let history = arrivals(&proving_ground, HISTORY, &["/robots.txt", "/", "/b", "/c"]);
    let edge = arrivals(
        &proving_ground,
        FETCH_EDGE,
        &[
            "/robots.txt",
            "/redirect/hops/2",
            "/redirect/hops/1",
            "/redirect/hops/0",
            "/refresh/script",
            "/target",
        ],
    );
    let popup = arrivals(
        &proving_ground,
        POPUP_HOST,
        &["/robots.txt", "/popup", "/popup/opened"],
    );
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            failed,
            (history.len(), edge.len(), popup.len()),
            (too_soon(&history), too_soon(&edge), too_soon(&popup)),
        ),
        (
            &json!("finished"),
            Vec::<Value>::new(),
            (4, 6, 3),
            (Vec::new(), Vec::new(), Vec::new()),
        ),
        "{history:?} {edge:?} {popup:?}"
    );
}
