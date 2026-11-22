//! The light-browser tier (spec #579; ADRs 0020 and 0093), on Foxhound's
//! executable against the Proving ground, Chrome for Testing and
//! Lightpanda, laid out in the test's Browser cache from the tests' copy
//! (`ci/lightpanda-fetch.sh`): where `fetch.light_browser` allows it, a
//! Label whose script-rendered pages match Chrome's on Lightpanda is
//! learned onto `light_browser`, whose requests Lightpanda sends from its
//! own process through the engine's bridge; a Label whose pages need
//! pixels stays on the browser; and a call the tier cannot serve
//! escalates under `auto` and fails with `browser_required` under a
//! forced tier. The tier table cases on this tier (spec #582) take each
//! row's call in turn, forced and under `auto`. Lightpanda's bridge sends
//! past any proxy, so a Run that sets one and allows the tier fails its
//! start, nothing reaching the site (#1640).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache_with_lightpanda};
use serde_json::{Value, json};

use crate::fetch_tiers::{
    engine_on, order, order_methods, ordering, requests_for, settings, tiers,
};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 771;

const HOST: &str = "tiers.test";

/// Seeds on [`HOST`], each path with its Label.
fn seeds(proving_ground: &ProvingGround, seeds: &[(&str, &str)]) -> Vec<Value> {
    seeds
        .iter()
        .map(|(path, label)| json!({ "url": proving_ground.url(HOST, path), "label": label }))
        .collect()
}

/// Each `fetch_tier` Provenance entry of `run_id` of a Label in `labels`,
/// as `<label> <tier> <chosen>`, with the call of an escalation.
fn chosen(env: &TestEnv, run_id: &str, labels: &[&str]) -> Vec<String> {
    tiers(env, run_id)
        .iter()
        .filter(|entry| labels.iter().any(|label| entry[0] == *label))
        .map(|entry| {
            let line = format!(
                "{} {} {}",
                entry[0].as_str().unwrap(),
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            );
            match entry[4].as_str() {
                Some(call) if entry[2] == "escalation" => format!("{line} {call}"),
                _ => line,
            }
        })
        .collect()
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

/// The texts of the elements `open` starts in `html`, up to their closing
/// tag.
fn texts(html: &str, open: &str) -> Vec<String> {
    html.split(open)
        .skip(1)
        .filter_map(|rest| rest.split('<').next())
        .map(str::to_owned)
        .collect()
}

/// The path of `url`.
fn path_of(url: &str) -> String {
    format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default())
}

/// The engine, its Browser cache holding Chrome for Testing and Lightpanda.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache_with_lightpanda(env)
        .expect("Playwright's Chrome for Testing and the tests' Lightpanda are installed");
    engine_on(env, &cache)
}

/// Under `auto`, a Run that allows the light-browser tier and sends its
/// HTTP from the browser (`fetch.http_from: browser`) samples each
/// Label on `http`, which Provenance records as `browser_fetch`, its
/// document sent from the Session's browser (issue 2409), on
/// `light_browser` and on `browser`: a listing its script
/// fills differs over `http` and matches on Lightpanda, so its Label moves
/// to `light_browser`, where its next request is served from the
/// document Lightpanda's run of its script left, Lightpanda sending it
/// from its own process; a page whose text only painted pixels give
/// differs on Lightpanda too, so its Label stays on the browser.
///
/// Covers: story:5-120, story:5-114, setting:fetch.light_browser, operation:page.formats
#[test]
fn a_label_whose_sample_matches_chromes_runs_on_lightpanda_and_one_that_differs_stays_on_the_browser()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled: Vec<(String, Value, Value)> = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "light",
        &seeds(
            &proving_ground,
            &[
                ("/blog/1", "listing"),
                ("/blog/2", "listing"),
                ("/blog/3", "listing"),
                ("/blog/4", "listing"),
                ("/painted/1", "painted"),
                ("/painted/2", "painted"),
                ("/painted/3", "painted"),
                ("/painted/4", "painted"),
            ],
        ),
        &json!({ "listing": {}, "painted": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "light_browser": true, "http_from": "browser" } }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            let data = if handle["crawl_request"]["label"] == "listing" {
                json!(texts(&html, "<li class=\"item\">"))
            } else {
                json!(texts(&html, "<p class=\"painted\">"))
            };
            handled.push((
                path_of(handle["crawl_request"]["url"].as_str().unwrap()),
                handle["tier"].clone(),
                data.clone(),
            ));
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "read": data } }] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let sampled = |label: &str| {
        let mut lines = Vec::new();
        for _ in 0..3 {
            for tier in ["browser_fetch", "light_browser", "browser"] {
                lines.push(format!("{label} {tier} sample"));
            }
        }
        lines
    };
    let lightpanda_sent = observations.to_string().contains("Lightpanda/");
    let fourth: Vec<(String, Value)> = handled
        .iter()
        .filter(|(path, _, _)| path.ends_with("/4"))
        .map(|(path, tier, _)| (path.clone(), tier.clone()))
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            chosen(&env, "light", &["listing"]),
            chosen(&env, "light", &["painted"]),
            fourth,
            handled
                .iter()
                .filter(|(path, tier, _)| path == "/painted/1" && tier != "http")
                .map(|(_, tier, data)| (tier.clone(), data.clone()))
                .collect::<Vec<_>>(),
            lightpanda_sent,
        ),
        (
            &json!("finished"),
            [
                sampled("listing"),
                vec!["listing light_browser compared".to_owned()]
            ]
            .concat(),
            [
                sampled("painted"),
                vec!["painted browser compared".to_owned()]
            ]
            .concat(),
            vec![
                ("/blog/4".to_owned(), json!("light_browser")),
                ("/painted/4".to_owned(), json!("browser")),
            ],
            vec![
                (json!("light_browser"), json!(["alpha 0"])),
                (json!("browser"), json!(["alpha 255"])),
            ],
            true,
        ),
        "{observations}"
    );
}

/// A call the light-browser tier cannot serve exactly, `page.evaluate`, a
/// script of the handler's, escalates its request to the browser under
/// `auto`, where it is served; under a forced `light_browser`, a gesture
/// fails with `browser_required` naming `fetch.tier` and the call, and
/// nothing escalates (ADR 0093). Under `auto`, its HTTP leaves from the
/// browser (`fetch.http_from: browser`).
///
/// Covers: story:5-120, story:5-115, error:browser_required, operation:page.evaluate, operation:locator.click
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one test drives two crawls, under `auto` and forced, whose answers it compares together"
)]
fn a_call_the_light_browser_cannot_serve_escalates_under_auto_and_fails_when_forced() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut evaluated = Vec::new();
    let learned = crawl_declaring(
        &mut engine,
        "light-auto",
        &seeds(
            &proving_ground,
            &[
                ("/blog/1", "listing"),
                ("/blog/2", "listing"),
                ("/blog/3", "listing"),
                ("/blog/4", "listing"),
            ],
        ),
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "light_browser": true, "http_from": "browser" } }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            if handle["crawl_request"]["url"]
                .as_str()
                .unwrap()
                .ends_with("/blog/4")
            {
                let reply = inside.call(
                    "page.evaluate",
                    &json!({ "page_id": handle["page_id"], "function": "() => document.querySelectorAll('li.item').length" }),
                );
                evaluated.push((
                    handle["tier"].clone(),
                    reply.get("error").map_or_else(
                        || reply["result"].clone(),
                        |error| error["data"]["code"].clone(),
                    ),
                ));
                if reply.get("error").is_some() {
                    return json!({ "error": reply["error"] });
                }
            }
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "items": texts(&html, "<li class=\"item\">") } }] } })
        },
    );

    let mut clicked = Vec::new();
    let forced = crawl_declaring(
        &mut engine,
        "light-forced",
        &seeds(&proving_ground, &[("/blog/5", "listing")]),
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "light_browser", "light_browser": true } }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            let reply = inside.call(
                "locator.click",
                &json!({ "locator": { "page_id": handle["page_id"], "selector": "li.item" } }),
            );
            clicked.push((
                handle["tier"].clone(),
                texts(&html, "<li class=\"item\">").len(),
                reply["error"]["data"]["code"].clone(),
                reply["error"]["data"]["call"].clone(),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let learned_lines = chosen(&env, "light-auto", &["listing"]);
    assert_eq!(
        (
            &learned["result"]["lifecycle"],
            learned_lines[9..].to_vec(),
            evaluated.len(),
            evaluated.last().cloned(),
            &forced["result"]["lifecycle"],
            chosen(&env, "light-forced", &["listing"]),
            clicked,
        ),
        (
            &json!("finished"),
            vec![
                "listing light_browser compared".to_owned(),
                "listing browser escalation page.evaluate".to_owned(),
            ],
            2,
            Some((json!("browser"), json!({ "value": 3 }))),
            &json!("finished"),
            vec!["listing light_browser forced".to_owned()],
            vec![(
                json!("light_browser"),
                3,
                json!("browser_required"),
                json!("locator.click")
            )],
        )
    );
}

/// A crawl request `POST /orders` with a body, or a `GET` with that body,
/// on a forced `light_browser` tier: Lightpanda's run loads a URL with a
/// `GET` and no body, so the tier refuses the request before anything is
/// sent, and it ends `failed` with `browser_required` naming the tier and
/// what of the crawl request it cannot send, never as a bodiless `GET`
/// (issue 2471).
///
/// Covers: story:6-212, story:5-120, setting:fetch.light_browser, error:browser_required
#[rstest::rstest]
#[case::a_post("POST", "crawl_request.method")]
#[case::a_get_with_a_body("GET", "crawl_request.body")]
fn a_crawl_request_with_a_method_or_body_on_a_forced_light_browser_fails_with_browser_required_and_sends_nothing(
    #[case] method: &str,
    #[case] unsendable: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let mut request = order(&proving_ground);
    request["method"] = json!(method);

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "light-post-forced",
        &[request],
        &json!({ "order": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "light_browser", "light_browser": true },
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut ordering(&mut seen),
    );
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "light-post-forced" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let ended: Vec<Value> = answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            let details = &outcome["error"]["details"];
            json!([
                outcome["kind"],
                outcome["error"]["code"],
                details["key"],
                details["tier"],
                details["call"]
            ])
        })
        .collect();
    assert_eq!(
        (order_methods(&proving_ground), seen, ended),
        (
            Vec::<String>::new(),
            Vec::new(),
            vec![json!([
                "failed",
                "browser_required",
                "fetch.tier",
                "light_browser",
                unsendable
            ])],
        )
    );
}

/// Under `auto`, a `POST` crawl request with a body on a Label its samples
/// moved onto the light-browser tier escalates to the browser before
/// Lightpanda sends anything: the site receives it only as that `POST`,
/// from Chrome, and Provenance names the crawl request's method as what
/// escalated it (issue 2471). Its HTTP leaves from the browser
/// (`fetch.http_from: browser`).
///
/// Covers: story:6-212, story:6-259, story:5-120, setting:fetch.light_browser
#[test]
fn a_post_crawl_request_on_a_label_learned_onto_the_light_browser_escalates_to_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let mut sent = seeds(
        &proving_ground,
        &[
            ("/blog/1", "listing"),
            ("/blog/2", "listing"),
            ("/blog/3", "listing"),
        ],
    );
    sent.push(json!({
        "url": proving_ground.url(HOST, "/blog/4"),
        "label": "listing",
        "method": "POST",
        "body": "cmVhZA==",
    }));

    let mut handled = Vec::new();
    crawl_declaring(
        &mut engine,
        "light-post-auto",
        &sent,
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "auto", "light_browser": true, "http_from": "browser" },
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            handled.push((
                path_of(handle["crawl_request"]["url"].as_str().unwrap()),
                handle["tier"].clone(),
            ));
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "items": texts(&html, "<li class=\"item\">") } }] } })
        },
    );
    assert!(engine.finished());

    let fourth: Vec<(String, bool)> = requests_for(&proving_ground, HOST, "/blog/4")
        .iter()
        .map(|request| {
            (
                request.method.clone(),
                request
                    .header("user-agent")
                    .is_some_and(|agent| agent.contains("Lightpanda/")),
            )
        })
        .collect();
    let lines = chosen(&env, "light-post-auto", &["listing"]);
    assert_eq!(
        (
            fourth,
            handled
                .iter()
                .filter(|(path, _)| path == "/blog/4")
                .cloned()
                .collect::<Vec<_>>(),
            lines[9..].to_vec(),
        ),
        (
            vec![("POST".to_owned(), false)],
            vec![("/blog/4".to_owned(), json!("browser"))],
            vec!["listing browser escalation crawl_request.method".to_owned()],
        )
    );
}

/// The rows of spec #582's served-call table a handler's call reaches on
/// the light-browser tier, in the order of the requests that make them.
/// The question and layout rows (`boundingBox`, `screenshot`) have no call
/// in the catalog yet (#1091), so only the table's own cases hold them.
const ROWS: [&str; 9] = [
    "formats", "rendered", "absent", "response", "script", "click", "goto", "request", "close",
];

/// What a case reads of a call's result.
type Part = fn(&Value) -> Value;

/// The call of the row `row` on the handler's page `page_id`, whose
/// static listing is at `products`: its method, its params and what a
/// case reads of its result. The waits give up after 100 ms, so in the
/// browser, which serves them, they answer `deadline_exceeded`.
fn row_call(row: &str, page_id: &Value, products: &str) -> (&'static str, Value, Part) {
    let locator = |selector: &str| json!({ "page_id": page_id, "selector": selector });
    let served: Part = |_| json!("served");
    match row {
        "formats" => (
            "page.formats",
            json!({ "page_id": page_id, "formats": { "markdown": true } }),
            |result| result["main_content"].clone(),
        ),
        "rendered" => (
            "locator.wait_for_count",
            json!({ "locator": locator("li.item"), "count": { "min": 1 } }),
            |result| result["count"].clone(),
        ),
        "absent" => (
            "locator.wait_for",
            json!({ "locator": locator("#absent"), "state": "attached", "timeout": 100 }),
            served,
        ),
        "response" => (
            "page.wait_for_response",
            json!({
                "page_id": page_id,
                "url": { "kind": "glob", "pattern": "**/never" },
                "timeout": 100,
            }),
            served,
        ),
        "script" => (
            "page.evaluate",
            json!({
                "page_id": page_id,
                "function": "() => document.querySelectorAll('li.item').length",
            }),
            |result| result["value"].clone(),
        ),
        "click" => ("locator.click", json!({ "locator": locator("h1") }), served),
        "goto" => (
            "page.goto",
            json!({ "page_id": page_id, "url": products }),
            |result| result["response"]["status"].clone(),
        ),
        "request" => (
            "page.request",
            json!({ "page_id": page_id, "url": products }),
            |result| result["status"].clone(),
        ),
        _ => ("page.close", json!({ "page_id": page_id }), served),
    }
}

/// What each row's request saw, by row: each tier its handler ran on, with
/// its call's answer.
type ByRow = std::collections::BTreeMap<String, Vec<(Value, Value)>>;

/// A handler that makes, on the page `/blog/<n>` whose `n` is `first` or
/// more, the call of row `n - first` of [`ROWS`], and records its answer
/// in `seen`: a call its tier does not serve ends the run with its error,
/// as an SDK's handler raising it would; any other answer, a served wait's
/// `deadline_exceeded` included, ends it with no Record. Earlier pages,
/// the samples, make no row's call and answer the listing's entries as a
/// Record, which only a tier that ran its script reads.
fn rows_handler(
    first: usize,
    seen: &mut ByRow,
) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
        let number: usize = url.rsplit('/').next().unwrap().parse().unwrap();
        let Some(row) = number.checked_sub(first).and_then(|index| ROWS.get(index)) else {
            let items = texts(&raw_html(inside, handle), "<li class=\"item\">");
            return json!({ "result": { "records": [{ "dataset": "pages", "data": { "items": items } }] } });
        };
        let products = url.replace(&format!("/blog/{number}"), "/products/1");
        let (method, params, part) = row_call(row, &handle["page_id"], &products);
        let reply = inside.call(method, &params);
        let answer = crate::served_calls::read(&reply, part);
        seen.entry((*row).to_owned())
            .or_default()
            .push((handle["tier"].clone(), answer.clone()));
        if answer == "browser_required" || answer == "attempt_ended" {
            json!({ "error": reply["error"] })
        } else {
            json!({ "result": { "records": [] } })
        }
    }
}

/// Seeds `/blog/<n>` of the Label `listing` for each `n` of `numbers`, with
/// their paths.
fn listing(
    proving_ground: &ProvingGround,
    numbers: std::ops::Range<usize>,
) -> (Vec<Value>, Vec<String>) {
    let paths: Vec<String> = numbers.map(|number| format!("/blog/{number}")).collect();
    let pages: Vec<(&str, &str)> = paths
        .iter()
        .map(|path| (path.as_str(), "listing"))
        .collect();
    (seeds(proving_ground, &pages), paths)
}

/// Tier table cases on a forced light-browser tier: each row's call on a
/// script-rendered page is served exactly where the table says, its
/// content read without layout, and any other fails with
/// `browser_required`, the request ending `failed` in one attempt, nothing
/// escalating.
///
/// Covers: story:6-261, story:6-259, story:5-120, setting:fetch.light_browser, error:browser_required, operation:page.formats, operation:locator.wait_for_count, operation:locator.wait_for, operation:page.wait_for_response, operation:page.evaluate, operation:locator.click, operation:page.goto, operation:page.request, operation:page.close
#[test]
fn under_a_forced_light_browser_each_row_is_served_as_the_table_says_or_refused() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let first = 11;
    let (seeds, _) = listing(&proving_ground, first..first + ROWS.len());

    let mut seen = ByRow::new();
    crawl_declaring(
        &mut engine,
        "light-table-forced",
        &seeds,
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "light_browser", "light_browser": true } }),
        ),
        &mut rows_handler(first, &mut seen),
    );
    let mut ended = crate::served_calls::ended(&mut engine, "light-table-forced");
    assert!(engine.finished());

    let light = |answer: Value| vec![(json!("light_browser"), answer)];
    let expected_seen = ByRow::from([
        ("formats".to_owned(), light(json!("found"))),
        ("rendered".to_owned(), light(json!(3))),
        ("absent".to_owned(), light(json!("browser_required"))),
        ("response".to_owned(), light(json!("browser_required"))),
        ("script".to_owned(), light(json!("browser_required"))),
        ("click".to_owned(), light(json!("browser_required"))),
        ("goto".to_owned(), light(json!("browser_required"))),
        ("request".to_owned(), light(json!(200))),
        ("close".to_owned(), light(json!("served"))),
    ]);
    let mut expected_ended = vec![
        json!(["/blog/11", "succeeded", null, 1]),
        json!(["/blog/12", "succeeded", null, 1]),
        json!(["/blog/13", "failed", "browser_required", 1]),
        json!(["/blog/14", "failed", "browser_required", 1]),
        json!(["/blog/15", "failed", "browser_required", 1]),
        json!(["/blog/16", "failed", "browser_required", 1]),
        json!(["/blog/17", "failed", "browser_required", 1]),
        json!(["/blog/18", "succeeded", null, 1]),
        json!(["/blog/19", "succeeded", null, 1]),
    ];
    ended.sort_by_key(ToString::to_string);
    expected_ended.sort_by_key(ToString::to_string);
    assert_eq!((seen, ended), (expected_seen, expected_ended));
}

/// The same rows under `auto`, on a Label its samples moved onto the
/// light-browser tier: a call the tier serves is answered there, and one
/// it cannot serve exactly ends that run and runs the same handler again
/// in the browser in the same attempt, where it is served; every request
/// ends `succeeded` in one attempt, and Provenance names each call that
/// escalated. Its HTTP leaves from the browser (`fetch.http_from:
/// browser`).
///
/// Covers: story:6-261, story:6-259, story:5-120, story:5-115, setting:fetch.light_browser, error:attempt_ended, operation:page.formats, operation:locator.wait_for_count, operation:locator.wait_for, operation:page.wait_for_response, operation:page.evaluate, operation:locator.click, operation:page.goto, operation:page.request, operation:page.close
#[test]
fn under_auto_a_call_the_light_browser_cannot_serve_escalates_its_request_once() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let first = 4;
    let (seeds, _) = listing(&proving_ground, 1..first + ROWS.len());

    let mut seen = ByRow::new();
    crawl_declaring(
        &mut engine,
        "light-table-auto",
        &seeds,
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "light_browser": true, "http_from": "browser" } }),
        ),
        &mut rows_handler(first, &mut seen),
    );
    let ended = crate::served_calls::ended(&mut engine, "light-table-auto");
    assert!(engine.finished());

    let light = |answer: Value| vec![(json!("light_browser"), answer)];
    let escalated = |answer: Value| {
        vec![
            (json!("light_browser"), json!("attempt_ended")),
            (json!("browser"), answer),
        ]
    };
    let expected_seen = ByRow::from([
        ("formats".to_owned(), light(json!("found"))),
        ("rendered".to_owned(), light(json!(3))),
        ("absent".to_owned(), escalated(json!("deadline_exceeded"))),
        ("response".to_owned(), escalated(json!("deadline_exceeded"))),
        ("script".to_owned(), escalated(json!(3))),
        ("click".to_owned(), escalated(json!("served"))),
        ("goto".to_owned(), escalated(json!(200))),
        ("request".to_owned(), light(json!(200))),
        ("close".to_owned(), light(json!("served"))),
    ]);
    let expected_lines: Vec<String> = [
        "listing light_browser compared",
        "listing light_browser compared",
        "listing light_browser compared",
        "listing browser escalation locator.wait_for",
        "listing light_browser compared",
        "listing browser escalation page.wait_for_response",
        "listing light_browser compared",
        "listing browser escalation page.evaluate",
        "listing light_browser compared",
        "listing browser escalation locator.click",
        "listing light_browser compared",
        "listing browser escalation page.goto",
        "listing light_browser compared",
        "listing light_browser compared",
    ]
    .map(str::to_owned)
    .to_vec();
    let lines = chosen(&env, "light-table-auto", &["listing"]);
    let not_once: Vec<&Value> = ended
        .iter()
        .filter(|outcome| outcome[1] != "succeeded" || outcome[3] != 1)
        .collect();
    assert_eq!(
        (seen, not_once, lines.get(9..).map(<[String]>::to_vec)),
        (expected_seen, Vec::<&Value>::new(), Some(expected_lines))
    );
}

/// A Run that allows the light-browser tier and sets a proxy fails its
/// start with `capability_unavailable` naming `fetch.light_browser`:
/// Lightpanda's bridge connects to sites directly, so its requests would
/// show the site the machine's own address the proxy hides (#1640). The
/// site receives no request from Lightpanda, through the proxy or not.
///
/// Covers: setting:fetch.light_browser, setting:proxy.server, error:capability_unavailable
#[test]
fn a_run_allowing_the_light_browser_beside_a_proxy_fails_its_start() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let proxy = format!(
        "http://{}",
        proving_ground.description().fixtures.proxies["open"]
    );
    let mut params = json!({
        "run_id": "light-proxied",
        "seeds": seeds(&proving_ground, &[("/blog/1", "listing")]),
        "labels": { "listing": {} },
        "settings": settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "light_browser", "light_browser": true },
                "proxy": { "server": proxy },
            }),
        ),
    });
    params["settings"]["crawl"]["handler"] = json!({ "concurrency": 1 });

    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": params,
    }));
    let refused = loop {
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["method"] == "crawl_request.handle" {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
            }));
        } else if message["id"] == "start" {
            if message.get("error").is_some() {
                break message["error"]["data"].clone();
            }
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "finish", "method": "run.finish",
                "params": { "run_id": "light-proxied" },
            }));
        } else if message["id"] == "finish" {
            break Value::Null;
        }
    };
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let from_lightpanda = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| {
            serde_json::to_string(request)
                .unwrap()
                .contains("Lightpanda/")
        })
        .count();
    let relayed = observations["relays"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|relay| {
            relay["target"]
                .as_str()
                .is_some_and(|target| target.contains(HOST))
        })
        .count();
    assert_eq!(
        (&refused["code"], &refused["key"], from_lightpanda, relayed,),
        (
            &json!("capability_unavailable"),
            &json!("fetch.light_browser"),
            0,
            0
        ),
        "the requests Lightpanda sent reached the site past the proxy when the start was not refused"
    );
}
