//! Crawl conformance cases, the tier lines (spec #579's `tiers`; ADRs
//! 0020, 0023 and 0093), on Foxhound's executable against the Proving
//! ground and Chrome for Testing: one handler runs unchanged on every tier.
//! Under a `fetch.tier: auto` the Run sets, a request is served over HTTP
//! from its document (the built-in default still hands a handler its
//! request alone until the tier choice, #768, is built), and a call that tier cannot serve exactly, a wait
//! for content a script-rendered page's document lacks, escalates the
//! request to the browser, where the same handler runs again, neither a
//! retry nor a failure; under a forced `http` tier the same call fails with
//! `browser_required`; `fetch.tier` set on a Label forces that Label's
//! requests only, and Provenance records each attempt's tier and how it
//! was chosen; `page.request` on the document tier goes through the
//! navigation gate and never escalates.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::PathBuf;
use std::time::Duration;

use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 767;

/// A static listing: its products are in its own markup.
const STATIC_HOST: &str = "catalog.test";
const STATIC_PATH: &str = "/catalog/1";

/// A script-rendered feed: its items are loaded by its script.
const SCRIPTED_HOST: &str = "rendered.test";
const SCRIPTED_PATH: &str = "/feed";

/// What the handler waits for: a listing's products or a feed's items.
const ITEMS: &str = "li.product, li.item";

/// An engine whose pages launch Chrome for Testing from the test's cache.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    Attached::start_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl that reaches every host of `proving_ground`,
/// with `more` merged in.
fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let mut settings = json!({ "network": { "resolve": resolve } });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    settings
}

/// One handler request as the handler saw it: the URL, the attempt, the
/// tier, and what its wait for the items answered.
#[derive(Debug, PartialEq)]
struct Handled {
    url: String,
    attempt: Value,
    tier: Value,
    waited: Value,
}

/// The one handler: it waits for at least one item, then reads the page's
/// Markdown, and answers no Record; a wait that fails ends the attempt
/// with its error, as an SDK's handler raising it would.
fn handler(seen: &mut Vec<Handled>) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        let page_id = handle["page_id"].clone();
        let waited = inside.call(
            "locator.wait_for_count",
            &json!({
                "locator": { "page_id": page_id, "selector": ITEMS },
                "count": { "min": 1 },
            }),
        );
        let answer = if let Some(error) = waited.get("error") {
            json!({ "error": error })
        } else {
            json!({ "result": { "records": [] } })
        };
        seen.push(Handled {
            url: handle["crawl_request"]["url"].as_str().unwrap().to_owned(),
            attempt: handle["attempt"].clone(),
            tier: handle["tier"].clone(),
            waited: waited
                .get("result")
                .map_or_else(|| waited["error"]["data"].clone(), Clone::clone),
        });
        answer
    }
}

/// The count of matches a wait answered, else its error's code.
fn waited(handled: &Handled) -> Value {
    if handled.waited["count"].is_u64() {
        json!(handled.waited["count"].as_u64().unwrap() > 0)
    } else {
        handled.waited["code"].clone()
    }
}

/// The Provenance entries of the Run `run_id` whose action is `action`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's Provenance log from outside the engine, on no async worker"
)]
fn provenance(env: &TestEnv, run_id: &str, action: &str) -> Vec<Value> {
    let file: PathBuf = env
        .home()
        .join("0.0")
        .join("runs")
        .join(run_id)
        .join("trace")
        .join("provenance.jsonl");
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == action)
        .collect()
}

/// Each `fetch_tier` Provenance entry of `run_id`: its Label, tier, how
/// it was chosen, by whom when forced, and the call that escalated it.
fn tiers(env: &TestEnv, run_id: &str) -> Vec<Value> {
    provenance(env, run_id, "fetch_tier")
        .iter()
        .map(|entry| {
            json!([
                entry["label"],
                entry["tier"],
                entry["chosen"],
                entry["forced_by"],
                entry["call"]
            ])
        })
        .collect()
}

/// The kind each request ended with, by URL, and its error's code.
fn outcomes(engine: &mut Attached, run_id: &str) -> Vec<(String, Value, Value)> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let mut outcomes: Vec<(String, Value, Value)> = answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                outcome["kind"].clone(),
                outcome["error"]["code"].clone(),
            )
        })
        .collect();
    outcomes.sort_by(|left, right| left.0.cmp(&right.0));
    outcomes
}

/// One handler under `fetch.tier: auto`: the static listing is served over
/// HTTP, its wait answered from its document, while the script-rendered
/// feed's wait, which its document cannot meet, escalates the request to
/// the browser, where the same handler runs again in the same attempt and
/// its wait is met; both end `succeeded`, and Provenance records `http`
/// chosen by the engine for both, then `browser` by escalation, naming the
/// call.
///
/// Covers: story:5-115, story:5-118, operation:locator.wait_for_count, error:attempt_ended
#[test]
fn one_handler_serves_a_static_page_over_http_and_escalates_a_scripted_one_under_auto() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let listing = proving_ground.url(STATIC_HOST, STATIC_PATH);
    let feed = proving_ground.url(SCRIPTED_HOST, SCRIPTED_PATH);

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "tiers-auto",
        &[
            json!({ "url": listing, "label": "page" }),
            json!({ "url": feed, "label": "page" }),
        ],
        &json!({ "page": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut handler(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-auto");
    assert!(engine.finish());

    let seen: Vec<_> = seen
        .iter()
        .map(|handled| {
            (
                handled.url.clone(),
                handled.attempt.clone(),
                handled.tier.clone(),
                waited(handled),
            )
        })
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            ended,
            tiers(&env, "tiers-auto"),
        ),
        (
            &json!("finished"),
            vec![
                (listing.clone(), json!(1), json!("http"), json!(true)),
                (
                    feed.clone(),
                    json!(1),
                    json!("http"),
                    json!("attempt_ended")
                ),
                (feed.clone(), json!(1), json!("browser"), json!(true)),
            ],
            vec![
                (listing, json!("succeeded"), Value::Null),
                (feed, json!("succeeded"), Value::Null),
            ],
            vec![
                json!(["page", "http", "engine", null, null]),
                json!(["page", "http", "engine", null, null]),
                json!([
                    "page",
                    "browser",
                    "escalation",
                    null,
                    "locator.wait_for_count"
                ]),
            ],
        )
    );
}

/// The same handler under a forced `http` tier: the static listing is
/// served as under `auto`, while the feed's wait fails with
/// `browser_required` naming `fetch.tier` and the call, nothing escalates
/// to the browser, and the request ends `failed` with it; Provenance
/// records both as forced by the Run.
///
/// Covers: story:5-115, story:5-118, operation:locator.wait_for_count, error:browser_required
#[test]
fn the_same_handler_under_a_forced_http_tier_fails_the_scripted_page_with_browser_required() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let listing = proving_ground.url(STATIC_HOST, STATIC_PATH);
    let feed = proving_ground.url(SCRIPTED_HOST, SCRIPTED_PATH);

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-forced",
        &[
            json!({ "url": listing, "label": "page" }),
            json!({ "url": feed, "label": "page" }),
        ],
        &json!({ "page": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "http" } })),
        &mut handler(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-forced");
    assert!(engine.finish());

    let refused = seen
        .iter()
        .find(|handled| handled.url == feed)
        .map(|handled| handled.waited.clone());
    assert_eq!(
        (
            seen.iter()
                .map(|handled| (handled.url.clone(), handled.tier.clone()))
                .collect::<Vec<_>>(),
            refused.map(|data| json!([data["code"], data["key"], data["tier"], data["call"]])),
            ended,
            tiers(&env, "tiers-forced"),
        ),
        (
            vec![
                (listing.clone(), json!("http")),
                (feed.clone(), json!("http"))
            ],
            Some(json!([
                "browser_required",
                "fetch.tier",
                "http",
                "locator.wait_for_count"
            ])),
            vec![
                (listing, json!("succeeded"), Value::Null),
                (feed, json!("failed"), json!("browser_required")),
            ],
            vec![
                json!(["page", "http", "forced", "run", null]),
                json!(["page", "http", "forced", "run", null]),
            ],
        )
    );
}

/// `fetch.tier` set on one Label forces its requests only: the feed under
/// the Label `list`, forced to `http`, fails with `browser_required` naming
/// that Label, while the same feed under the Label `page`, left at `auto`,
/// escalates to the browser; Provenance records the first as forced by its
/// Label.
///
/// Covers: story:5-118, error:browser_required
#[test]
fn fetch_tier_on_a_label_forces_that_label_only_and_provenance_records_it_forced() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let feed = proving_ground.url(SCRIPTED_HOST, SCRIPTED_PATH);
    let also = format!("{feed}?copy=1");

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-label",
        &[
            json!({ "url": feed, "label": "list" }),
            json!({ "url": also, "label": "page" }),
        ],
        &json!({
            "list": { "settings": { "fetch": { "tier": "http" } } },
            "page": {},
        }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut handler(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-label");
    assert!(engine.finish());

    let refused = seen
        .iter()
        .find(|handled| handled.url == feed)
        .map(|handled| json!([handled.waited["code"], handled.waited["label"]]));
    assert_eq!(
        (refused, ended, tiers(&env, "tiers-label")),
        (
            Some(json!(["browser_required", "list"])),
            vec![
                (feed, json!("failed"), json!("browser_required")),
                (also, json!("succeeded"), Value::Null),
            ],
            vec![
                json!(["list", "http", "forced", "label", null]),
                json!(["page", "http", "engine", null, null]),
                json!([
                    "page",
                    "browser",
                    "escalation",
                    null,
                    "locator.wait_for_count"
                ]),
            ],
        )
    );
}

/// The `access-policy` family's robots.txt host: its `*` group disallows
/// `/private/`.
const ROBOTS_HOST: &str = "1.access-policy.test";

/// `page.request` on a page the `http` tier serves under `auto` is sent
/// through the Session's HTTP transport and the navigation gate: an
/// allowed URL answers its response, whose body `response.body` keeps, a
/// URL robots.txt disallows is refused with `site_preferences_refused`
/// before it leaves, and neither escalates the request: no browser runs
/// it, and it ends `succeeded`.
///
/// Covers: story:5-115, operation:page.request, operation:response.body, error:site_preferences_refused
#[test]
fn page_request_on_a_document_tier_goes_through_the_gate_and_never_escalates() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seed = proving_ground.url(ROBOTS_HOST, "/catalog/1");
    let allowed = proving_ground.url(ROBOTS_HOST, "/catalog/2");
    let disallowed = proving_ground.url(ROBOTS_HOST, "/private/catalog/3");

    let mut answers = Vec::new();
    let mut tiers_seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-request",
        &[json!({ "url": seed, "label": "page" })],
        &json!({ "page": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            tiers_seen.push(handle["tier"].clone());
            let page_id = handle["page_id"].clone();
            let got = inside.call(
                "page.request",
                &json!({ "page_id": page_id, "url": allowed }),
            );
            let body = inside.call(
                "response.body",
                &json!({ "page_id": page_id, "request_id": got["result"]["request_id"] }),
            );
            let refused = inside.call(
                "page.request",
                &json!({ "page_id": page_id, "url": disallowed }),
            );
            answers.push((got, body, refused));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = outcomes(&mut engine, "tiers-request");
    assert!(engine.finish());

    let (got, body, refused) = &answers[0];
    let requests = proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == ROBOTS_HOST)
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .filter(|target| target != "/robots.txt")
        .collect::<Vec<_>>();
    assert_eq!(
        (
            tiers_seen,
            got["result"]["status"].clone(),
            body["result"]["size"].as_u64().is_some_and(|size| size > 0),
            refused["error"]["data"]["code"].clone(),
            refused["error"]["data"]["reason"].clone(),
            requests,
            ended,
            tiers(&env, "tiers-request"),
        ),
        (
            vec![json!("http")],
            json!(200),
            true,
            json!("site_preferences_refused"),
            json!("robots_txt"),
            vec!["/catalog/1".to_owned(), "/catalog/2".to_owned()],
            vec![(seed, json!("succeeded"), Value::Null)],
            vec![json!(["page", "http", "engine", null, null])],
        ),
        "{answers:?}"
    );
}

/// A script-written app shell: its HTML holds an empty `#app`, which its
/// inline script fills before the page loads.
const SHELL_PATH: &str = "/shell";

/// A Label's declaration whose `session.page_check` expects an element the
/// CSS selector `selector` finds, its other settings `more`.
fn expecting(selector: &str, more: &Value) -> Value {
    let mut settings = json!({
        "session": { "page_check": { "expected_page": { "kind": "css", "selector": selector } } },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    json!({ "settings": settings })
}

/// A handler that records the URL and tier of each request it is given
/// and answers no Record.
fn recording(seen: &mut Vec<(String, Value)>) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |_, handle| {
        seen.push((
            handle["crawl_request"]["url"].as_str().unwrap().to_owned(),
            handle["tier"].clone(),
        ));
        json!({ "result": { "records": [] } })
    }
}

/// Under `fetch.tier: auto`, a page whose document fails its Label's
/// expected page over HTTP, the app shell whose heading only its script
/// writes, is escalated to the browser instead of failing: its handler is
/// given the browser's page only, where the expected page passes, and it
/// ends `succeeded`; a static listing whose document meets its Label's
/// expected page stays on HTTP. Provenance records the shell's escalation
/// as caused by `session.page_check`.
///
/// Covers: story:5-137, setting:session.page_check
#[test]
fn a_page_failing_its_labels_expected_page_over_http_escalates_under_auto_and_passes_in_the_browser()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let listing = proving_ground.url(STATIC_HOST, STATIC_PATH);
    let shell = proving_ground.url(SCRIPTED_HOST, SHELL_PATH);

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "expected-auto",
        &[
            json!({ "url": listing, "label": "list" }),
            json!({ "url": shell, "label": "app" }),
        ],
        &json!({
            "list": expecting("li.product", &json!({})),
            "app": expecting("#app h1.title", &json!({})),
        }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut recording(&mut seen),
    );
    let ended = outcomes(&mut engine, "expected-auto");
    assert!(engine.finish());

    seen.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            ended,
            tiers(&env, "expected-auto"),
        ),
        (
            &json!("finished"),
            vec![
                (listing.clone(), json!("http")),
                (shell.clone(), json!("browser")),
            ],
            vec![
                (listing, json!("succeeded"), Value::Null),
                (shell, json!("succeeded"), Value::Null),
            ],
            vec![
                json!(["list", "http", "engine", null, null]),
                json!(["app", "http", "engine", null, null]),
                json!(["app", "browser", "escalation", null, "session.page_check"]),
            ],
        )
    );
}

/// A page failing its Label's expected page fails with `page_check_failed`
/// where nothing escalates it: the app shell under a Label forcing `http`,
/// whose document lacks the heading, and the same shell under a Label
/// forcing `browser`, which expects an element its script never writes.
/// No handler runs and both end `failed` with it.
///
/// Covers: story:5-137, setting:session.page_check, error:page_check_failed
#[test]
fn a_page_failing_its_expected_page_fails_with_page_check_failed_under_a_forced_http_tier_and_on_the_browser_tier()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let shell = proving_ground.url(SCRIPTED_HOST, SHELL_PATH);
    let also = format!("{shell}?copy=1");

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "expected-forced",
        &[
            json!({ "url": shell, "label": "http" }),
            json!({ "url": also, "label": "browser" }),
        ],
        &json!({
            "http": expecting("#app h1.title", &json!({ "fetch": { "tier": "http" } })),
            "browser": expecting("#app h2.missing", &json!({ "fetch": { "tier": "browser" } })),
        }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut recording(&mut seen),
    );
    let ended = outcomes(&mut engine, "expected-forced");
    assert!(engine.finish());

    assert_eq!(
        (seen, ended, tiers(&env, "expected-forced")),
        (
            Vec::new(),
            vec![
                (shell, json!("failed"), json!("page_check_failed")),
                (also, json!("failed"), json!("page_check_failed")),
            ],
            vec![json!(["http", "http", "forced", "label", null])],
        )
    );
}
