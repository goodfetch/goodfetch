//! Sample comparison cases (spec #579's Testing Decisions; ADRs 0020,
//! 0023, 0103 and 0118), on Foxhound's executable against the Proving
//! ground and Chrome for Testing: under `fetch.tier: auto`, a sample is
//! judged on what each of its runs staged, its Records and its enqueues;
//! the browser's run commits whether or not they match; a listing whose
//! links only its script adds stays on the browser, as does a page a
//! script redirects; Records that differ only in whitespace match;
//! and `fetch.sample.count` below 3 is a weakened control.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings, tiers};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 768;

const HOST: &str = "tiers.test";

/// Seeds on [`HOST`], each path with its Label.
fn seeds(proving_ground: &ProvingGround, seeds: &[(&str, &str)]) -> Vec<Value> {
    seeds
        .iter()
        .map(|(path, label)| json!({ "url": proving_ground.url(HOST, path), "label": label }))
        .collect()
}

/// Each `fetch_tier` Provenance entry of `run_id` of a Label in `labels`,
/// as `<label> <tier> <chosen>`.
fn chosen(env: &TestEnv, run_id: &str, labels: &[&str]) -> Vec<String> {
    tiers(env, run_id)
        .iter()
        .filter(|entry| labels.iter().any(|label| entry[0] == *label))
        .map(|entry| {
            format!(
                "{} {} {}",
                entry[0].as_str().unwrap(),
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            )
        })
        .collect()
}

/// The Records the Run `run_id` committed, in order.
fn committed(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut statement = connection
        .prepare("SELECT data FROM records ORDER BY rowid")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect()
}

/// The path of `url`.
fn path_of(url: &str) -> String {
    format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default())
}

/// `crawl_request.push` of `data` into `pages` from the attempt `handle`.
fn push(inside: &mut Inside<'_>, handle: &Value, data: &Value) {
    let reply = inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": "pages",
            "data": data,
        }),
    );
    assert!(reply.get("result").is_some(), "{reply}");
}

/// The page's raw HTML, as served or as the DOM stands in the browser.
fn raw_html(inside: &mut Inside<'_>, handle: &Value) -> String {
    let reply = inside.call(
        "page.formats",
        &json!({ "page_id": handle["page_id"], "formats": { "html": { "kind": "raw" } } }),
    );
    reply["result"]["html"]
        .as_str()
        .unwrap_or_else(|| panic!("no HTML: {reply}"))
        .to_owned()
}

/// A listing whose links only its script adds enqueues nothing over HTTP
/// and three links in the browser, so its samples never match and its
/// Label stays on the browser; the pages it links, which stage the same
/// nothing on both tiers, are learned onto `http` (ADR 0103).
///
/// Covers: story:5-114, story:5-116, operation:crawl_request.enqueue, operation:page.formats
#[test]
fn a_listing_whose_links_appear_only_after_script_enqueues_nothing_over_http_and_stays_on_the_browser()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut enqueued: Vec<(String, Value, usize)> = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "sample-listing",
        &seeds(
            &proving_ground,
            &[
                ("/linked/1", "listing"),
                ("/linked/2", "listing"),
                ("/linked/3", "listing"),
                ("/linked/4", "listing"),
            ],
        ),
        &json!({ "listing": {}, "leaf": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            if handle["crawl_request"]["label"] == "listing" {
                let html = raw_html(inside, handle);
                let links: Vec<String> = html
                    .split("href=\"")
                    .skip(1)
                    .filter_map(|link| link.split('"').next())
                    .map(str::to_owned)
                    .collect();
                for link in &links {
                    inside.enqueue(handle, link, "leaf", None);
                }
                enqueued.push((
                    path_of(handle["crawl_request"]["url"].as_str().unwrap()),
                    handle["tier"].clone(),
                    links.len(),
                ));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let listing: Vec<(&str, Value, usize)> = ["/linked/1", "/linked/2", "/linked/3"]
        .into_iter()
        .flat_map(|path| [(path, json!("http"), 0), (path, json!("browser"), 3)])
        .chain([("/linked/4", json!("browser"), 3)])
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            enqueued
                .iter()
                .map(|(path, tier, count)| (path.as_str(), tier.clone(), *count))
                .collect::<Vec<_>>(),
            chosen(&env, "sample-listing", &["listing"]),
            chosen(&env, "sample-listing", &["leaf"])
                .iter()
                .filter(|line| line.ends_with("compared"))
                .all(|line| line == "leaf browser_fetch compared"),
        ),
        (
            &json!("finished"),
            listing,
            [
                "listing browser_fetch sample",
                "listing browser sample",
                "listing browser_fetch sample",
                "listing browser sample",
                "listing browser_fetch sample",
                "listing browser sample",
                "listing browser compared",
            ]
            .map(str::to_owned)
            .to_vec(),
            true,
        )
    );
}

/// A sample's discarded HTTP run teaches the Frontier nothing (ADR 0020's
/// #768 note): three script-rendered shells that differ only by a query
/// parameter serve one document over HTTP, which would teach that the
/// parameter has no effect and merge the fourth page, still pending, as a
/// `duplicate`. Only the browser's run commits, and it teaches nothing, so
/// the fourth page is crawled in the browser.
///
/// Covers: story:5-114, story:5-116
#[test]
fn a_samples_discarded_http_run_teaches_the_frontier_nothing() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let report = crawl_declaring(
        &mut engine,
        "sample-shells",
        &seeds(
            &proving_ground,
            &[
                ("/blog/1?page=1", "list"),
                ("/blog/1?page=2", "list"),
                ("/blog/1?page=3", "list"),
                ("/blog/1?page=4", "list"),
            ],
        ),
        &json!({ "list": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            let items = raw_html(inside, handle)
                .matches("<li class=\"item\">")
                .count();
            push(inside, handle, &json!({ "items": items }));
            json!({ "result": { "records": [] } })
        },
    );
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "sample-shells" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let mut outcomes: Vec<(String, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                path_of(outcome["request"]["url"].as_str().unwrap()),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            outcomes,
            chosen(&env, "sample-shells", &["list"]),
        ),
        (
            &json!("finished"),
            (1..=4)
                .map(|page| (
                    format!("/blog/1?page={page}"),
                    json!("succeeded"),
                    Value::Null
                ))
                .collect::<Vec<_>>(),
            [
                "list browser_fetch sample",
                "list browser sample",
                "list browser_fetch sample",
                "list browser sample",
                "list browser_fetch sample",
                "list browser sample",
                "list browser compared",
            ]
            .map(str::to_owned)
            .to_vec(),
        ),
        "{read}"
    );
}

/// A page whose Records differ between the tiers only in whitespace
/// matches, and its Label moves to `http`; one whose Records name their
/// tier never matches and stays on the browser. Either way each sample
/// commits its browser run's Records alone, and a request on `http` its
/// own (ADR 0103).
///
/// Covers: story:5-114, story:5-116, operation:crawl_request.push
#[test]
fn records_that_differ_only_in_whitespace_match_and_the_browsers_run_commits_either_way() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let report = crawl_declaring(
        &mut engine,
        "sample-spaced",
        &seeds(
            &proving_ground,
            &[
                ("/products/1", "spaced"),
                ("/products/2", "spaced"),
                ("/products/3", "spaced"),
                ("/products/4", "spaced"),
                ("/blog/1", "tiered"),
                ("/blog/2", "tiered"),
                ("/blog/3", "tiered"),
            ],
        ),
        &json!({ "spaced": {}, "tiered": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            let path = path_of(handle["crawl_request"]["url"].as_str().unwrap());
            let on_http = handle["tier"] == "http";
            let data = if handle["crawl_request"]["label"] == "spaced" {
                let title = if on_http {
                    format!("  {path}\n\t ")
                } else {
                    path.clone()
                };
                json!({ "path": path, "title": title })
            } else {
                json!({ "path": path, "tier": handle["tier"] })
            };
            push(inside, handle, &data);
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let mut records = committed(&env, "sample-spaced");
    records.sort_by_key(|record| record["path"].as_str().unwrap().to_owned());
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            chosen(&env, "sample-spaced", &["spaced", "tiered"])
                .into_iter()
                .filter(|line| line.ends_with("compared"))
                .collect::<Vec<_>>(),
            records,
        ),
        (
            &json!("finished"),
            vec!["spaced browser_fetch compared".to_owned()],
            vec![
                json!({ "path": "/blog/1", "tier": "browser" }),
                json!({ "path": "/blog/2", "tier": "browser" }),
                json!({ "path": "/blog/3", "tier": "browser" }),
                json!({ "path": "/products/1", "title": "/products/1" }),
                json!({ "path": "/products/2", "title": "/products/2" }),
                json!({ "path": "/products/3", "title": "/products/3" }),
                json!({ "path": "/products/4", "title": "  /products/4\n\t " }),
            ],
        )
    );
}

/// A page a script redirects ends its browser run on another URL than its
/// document came from, so its samples never match though they push the
/// same Record, and its Label stays on the browser (ADR 0118).
///
/// Covers: story:5-117, operation:page.wait_for_load_state
#[test]
fn a_script_redirect_keeps_the_label_on_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let report = crawl_declaring(
        &mut engine,
        "sample-moving",
        &seeds(
            &proving_ground,
            &[
                ("/moving/1", "moving"),
                ("/moving/2", "moving"),
                ("/moving/3", "moving"),
                ("/moving/4", "moving"),
            ],
        ),
        &json!({ "moving": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            let waited = inside.call(
                "page.wait_for_load_state",
                &json!({ "page_id": handle["page_id"], "state": "load" }),
            );
            assert!(waited.get("result").is_some(), "{waited}");
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "moved": true } }] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            chosen(&env, "sample-moving", &["moving"]),
        ),
        (
            &json!("finished"),
            [
                "moving browser_fetch sample",
                "moving browser sample",
                "moving browser_fetch sample",
                "moving browser sample",
                "moving browser_fetch sample",
                "moving browser sample",
                "moving browser compared",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    );
}

/// `fetch.sample.count` below 3 is a weakened control the Run report's
/// `weakened` names, and a Label takes that many samples: with one, its
/// first matching sample moves it to `http` (ADR 0023).
///
/// Covers: story:5-114, setting:fetch.sample.count
#[test]
fn a_sample_count_below_three_is_a_weakened_control() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let report = crawl_declaring(
        &mut engine,
        "sample-count",
        &seeds(
            &proving_ground,
            &[("/products/1", "page"), ("/products/2", "page")],
        ),
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "sample": { "count": 1 } } }),
        ),
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert!(engine.finished());

    let weakened: Vec<(Value, Value)> = report["result"]["weakened"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|weakened| weakened["key"] != "egress.private_ranges")
        .map(|weakened| (weakened["key"].clone(), weakened["value"].clone()))
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            weakened,
            chosen(&env, "sample-count", &["page"]),
        ),
        (
            &json!("finished"),
            vec![(json!("fetch.sample.count"), json!(1))],
            [
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch compared"
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    );
}

/// A choice that served `fetch.sample.refresh_after`'s requests is sampled
/// again: with one request, the Label's next request after the one its
/// choice served is a sample on both tiers again, neither a retry nor a
/// failure, every request succeeding at its first attempt (ADR 0020).
///
/// Covers: story:5-114, setting:fetch.sample.refresh_after
#[test]
fn a_choice_that_served_its_requests_is_sampled_again() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut attempts = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "sample-refresh",
        &seeds(
            &proving_ground,
            &[
                ("/products/1", "page"),
                ("/products/2", "page"),
                ("/products/3", "page"),
                ("/products/4", "page"),
                ("/products/5", "page"),
            ],
        ),
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": {
                "tier": "auto",
                "sample": { "refresh_after": { "requests": 1, "after": 86_400_000 } },
            } }),
        ),
        &mut |_, handle| {
            attempts.push(handle["attempt"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            attempts.iter().all(|attempt| *attempt == json!(1)),
            chosen(&env, "sample-refresh", &["page"]),
        ),
        (
            &json!("finished"),
            true,
            [
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch compared",
                "page browser_fetch sample",
                "page browser sample",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    );
}
