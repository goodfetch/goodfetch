//! Crawl conformance cases, the tier lines (spec #579's `tiers`; ADRs
//! 0020, 0023 and 0093), on Foxhound's executable against the Proving
//! ground and Chrome for Testing: one handler runs unchanged on every tier.
//! Under `fetch.tier: auto`, which is also the built-in default, the tier
//! is learned per
//! Label on a domain by sample comparison: the first three requests of a
//! Label on a domain are samples, each run on `http`, served from its
//! document, and then in the browser, whose run commits; a Label whose
//! samples all gave the same Records moves to `http`, one whose samples
//! differed stays on `browser`, and one whose samples disagree by first
//! path segment is split by it, a segment whose own samples disagree
//! staying on `browser`; the choice survives a resume. A call the `http`
//! tier cannot serve exactly, a wait for content a script-rendered page's
//! document lacks, escalates the request to the browser, where the same
//! handler runs again, neither a retry nor a failure; under a forced `http`
//! tier the same call fails with `browser_required`; `fetch.tier` set on a
//! Label forces that Label's requests only, and Provenance records each
//! attempt's tier and how it was chosen; `page.request` on the document
//! tier goes through the navigation gate and never escalates.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::PathBuf;
use std::time::Duration;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ObservedRequest;
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::attached_with;
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
pub(crate) fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    engine_on(env, &cache)
}

/// An engine whose pages launch Chrome for Testing from `cache`, a cache
/// a test prepared once, so a second engine reuses it while the first's
/// browser may still run.
pub(crate) fn engine_on(env: &TestEnv, cache: &std::path::Path) -> Attached {
    attached_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, cache);
    })
}

/// The settings of a crawl that reaches every host of `proving_ground`,
/// with `more` merged in.
pub(crate) fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    // A browser tier reaches the Proving ground through the egress relay,
    // which judges a mapping's answers as any other.
    let mut settings = json!({
        "network": { "resolve": resolve },
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
    });
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
pub(crate) fn provenance(env: &TestEnv, run_id: &str, action: &str) -> Vec<Value> {
    let file: PathBuf = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_PROVENANCE);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == action)
        .collect()
}

/// Each `fetch_tier` Provenance entry of `run_id`: its Label, tier, how
/// it was chosen, by whom when forced, and the call that escalated it.
pub(crate) fn tiers(env: &TestEnv, run_id: &str) -> Vec<Value> {
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
pub(crate) fn outcomes(engine: &mut Attached, run_id: &str) -> Vec<(String, Value, Value)> {
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

/// One handler under `fetch.tier: auto`, each page the first of its Label
/// on its domain and so a sample: the static listing is served over HTTP,
/// its wait answered from its document, then in the browser; the
/// script-rendered feed's wait, which its document cannot meet, ends its
/// HTTP run, and the same handler runs again in the browser in the same
/// attempt, where its wait is met; both end `succeeded`, and Provenance
/// records each run as a sample, the feed's browser run naming the call
/// its HTTP run could not serve.
///
/// A Run that sets no `fetch.tier` is served the same way: `auto` is the
/// built-in default, and no handler is given its request alone (issue 1602).
///
/// Covers: story:5-115, story:5-118, story:6-257, operation:locator.wait_for_count, error:attempt_ended
#[rstest::rstest]
#[case::set(json!({ "fetch": { "tier": "auto" } }))]
#[case::the_built_in_default(json!({}))]
fn one_handler_runs_a_sample_of_a_static_page_and_of_a_scripted_one_on_both_tiers_under_auto(
    #[case] tier: Value,
) {
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
        &settings(&proving_ground, &tier),
        &mut handler(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-auto");
    assert!(engine.finished());

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
                (listing.clone(), json!(1), json!("browser"), json!(true)),
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
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, null]),
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, "locator.wait_for_count"]),
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
    assert!(engine.finished());

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
/// is sampled on both tiers; Provenance records the first as forced by its
/// Label.
///
/// Covers: story:5-118, story:6-257, error:browser_required
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
    assert!(engine.finished());

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
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, "locator.wait_for_count"]),
            ],
        )
    );
}

/// The `access-policy` family's robots.txt host: its `*` group disallows
/// `/private/`.
const ROBOTS_HOST: &str = "1.access-policy.test";

/// `page.request` on a page the `http` tier serves is sent through the
/// Session's HTTP transport and the navigation gate: an allowed URL answers
/// its response, whose body `response.body` keeps, a URL robots.txt
/// disallows is refused with `site_preferences_refused` before it leaves,
/// and neither is refused as a call the tier cannot serve: no browser runs
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
        &settings(&proving_ground, &json!({ "fetch": { "tier": "http" } })),
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
    assert!(engine.finished());

    let (got, body, refused) = &answers[0];
    let requests = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == ROBOTS_HOST)
        .map(|request| request.target.as_str().to_owned())
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
            vec![json!(["page", "http", "forced", "run", null])],
        ),
        "{answers:?}"
    );
}

/// The `fetch-edge` family's host, whose `/redirect/cross-origin` answers
/// a 302 to `/target` on [`EDGE_OTHER_HOST`], another origin.
const EDGE_HOST: &str = "fetch-edge.test";

/// The `fetch-edge` family's HTTP/1.1 host: another origin than
/// [`EDGE_HOST`].
const EDGE_OTHER_HOST: &str = "h1.fetch-edge.test";

/// The requests `proving_ground` received for `path` on `host`.
pub(crate) fn requests_for(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
) -> Vec<ObservedRequest> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host.split(':').next() == Some(host) && request.target == path)
        .cloned()
        .collect()
}

/// The method of `request` and whether it carried each header of `names`.
fn carried(request: &ObservedRequest, names: &[&str]) -> (Value, Vec<bool>) {
    (
        json!(request.method),
        names
            .iter()
            .map(|name| request.header(name).is_some())
            .collect(),
    )
}

/// A `page.request` from a page the `http` tier serves follows its
/// redirects with Fetch's header rules, as the crawl request's own chain
/// does (ADR 0118): a `POST` a 302 turns into a `GET` arrives without its
/// body's headers, and `Authorization` is removed once the chain reaches
/// another origin, while a hop on the same origin keeps it.
///
/// Covers: story:5-124, operation:page.request
#[test]
fn page_request_on_a_document_tier_drops_authorization_across_origins_and_body_headers_after_a_get()
{
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"{EDGE_HOST}\" = ['{}']\n",
        proving_ground.description().ca.display()
    ));
    let mut engine = engine(&env);
    let https = |path: &str| {
        proving_ground
            .url(EDGE_HOST, path)
            .replacen("http://", "https://", 1)
    };
    let seed = proving_ground.url(STATIC_HOST, STATIC_PATH);
    let sent = |url: String| {
        json!({
            "url": url,
            "method": "POST",
            "headers": {
                "Authorization": "Bearer test-token",
                "Content-Type": "application/x-www-form-urlencoded",
            },
            "post_data": "cT0x",
        })
    };

    let mut statuses = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-request-redirects",
        &[json!({ "url": seed, "label": "page" })],
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "http" },
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut |inside, handle| {
            for url in [https("/redirect/cross-origin"), https("/redirect/302")] {
                let mut params = sent(url);
                params["page_id"] = handle["page_id"].clone();
                let got = inside.call("page.request", &params);
                statuses.push(got["result"]["status"].clone());
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let names = ["authorization", "content-type", "content-length"];
    let reached = |host: &str| {
        requests_for(&proving_ground, host, "/target")
            .iter()
            .map(|request| carried(request, &names))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        (statuses, reached(EDGE_OTHER_HOST), reached(EDGE_HOST)),
        (
            vec![json!(200), json!(200)],
            vec![(json!("GET"), vec![false, false, false])],
            vec![(json!("GET"), vec![true, false, false])],
        )
    );
}

/// A `page.request` from a page the `http` tier serves holds its
/// attempt's lease while it waits for its permit (ADR 0093): the wait,
/// its domain's delay between starts, lasts longer than the lease was
/// granted for, the lease is renewed through it, and the request is sent
/// and answered while the attempt still holds its lease, which then
/// commits, never timed out.
///
/// The lease is the 1 s handler timeout plus its 10 s margin, and the
/// delay 12 s, in real time: a test of the binary sets short durations
/// through settings (TESTING.md, "Time").
///
/// Covers: story:5-135, setting:timeout.handler, setting:politeness.domain.delay, operation:page.request
#[test]
fn a_served_pages_request_waiting_for_its_permit_holds_its_attempts_lease() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let url = |path: &str| proving_ground.url(STATIC_HOST, path);

    let mut statuses = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-lease",
        &[json!({ "url": url(STATIC_PATH), "label": "page" })],
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "http" },
                // A lease of 1 s plus the 10 s margin, shorter than the
                // delay.
                "timeout": { "handler": 1_000 },
                "politeness": { "domain": { "delay": 12_000 } },
                "crawl": { "retry": { "max": 0 } },
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut |inside, handle| {
            let got = inside.call(
                "page.request",
                &json!({ "page_id": handle["page_id"], "url": url("/catalog/2") }),
            );
            statuses.push(got["result"]["status"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let ended = outcomes(&mut engine, "tiers-lease");
    assert!(engine.finished());

    assert_eq!(
        (
            statuses,
            ended,
            requests_for(&proving_ground, STATIC_HOST, "/catalog/2").len(),
        ),
        (
            vec![json!(200)],
            vec![(url(STATIC_PATH), json!("succeeded"), Value::Null)],
            1,
        )
    );
}

/// The catalog's order endpoint: a `POST` whose form body names a product
/// answers a page naming it in `span.id`, and any other method a 404.
pub(crate) const ORDER_PATH: &str = "/orders";

/// `product=P-0001`, in Base64 as a crawl request carries its body.
pub(crate) const ORDER_BODY: &str = "cHJvZHVjdD1QLTAwMDE=";

/// The product an order's body names.
pub(crate) const ORDERED: &str = "P-0001";

/// A crawl request `POST /orders` with [`ORDER_BODY`] under the Label
/// `order`.
pub(crate) fn order(proving_ground: &ProvingGround) -> Value {
    json!({
        "url": proving_ground.url(STATIC_HOST, ORDER_PATH),
        "label": "order",
        "method": "POST",
        "body": ORDER_BODY,
    })
}

/// The methods `proving_ground` received `/orders` with.
pub(crate) fn order_methods(proving_ground: &ProvingGround) -> Vec<String> {
    requests_for(proving_ground, STATIC_HOST, ORDER_PATH)
        .into_iter()
        .map(|request| request.method)
        .collect()
}

/// A handler that reads the product the page names by script, which only
/// a browser serves: each run's tier with that product, or with the code
/// of the error the call raised, which ends the run.
pub(crate) fn ordering(
    seen: &mut Vec<(Value, Value)>,
) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        let reply = inside.call(
            "page.evaluate",
            &json!({
                "page_id": handle["page_id"],
                "function": "() => document.querySelector('span.id')?.textContent ?? null",
            }),
        );
        if let Some(error) = reply.get("error") {
            seen.push((handle["tier"].clone(), error["data"]["code"].clone()));
            return json!({ "error": error });
        }
        seen.push((handle["tier"].clone(), reply["result"]["value"].clone()));
        json!({ "result": { "records": [] } })
    }
}

/// A crawl request `POST /orders` with a body, on a forced `browser`
/// tier, reaches the site as that `POST` with that body: the browser's
/// own navigation carries the crawl request's method and body, as
/// Crawlee's and scrapy-playwright's do, and the page it opens is the
/// one that body orders (issue 2471).
///
/// Covers: story:6-212, story:5-118, operation:page.evaluate
#[test]
fn a_post_crawl_request_with_a_body_reaches_the_site_as_that_post_on_a_forced_browser_tier() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-post-browser",
        &[order(&proving_ground)],
        &json!({ "order": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "browser" },
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut ordering(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-post-browser");
    assert!(engine.finished());

    assert_eq!(
        (order_methods(&proving_ground), seen, ended),
        (
            vec!["POST".to_owned()],
            vec![(json!("browser"), json!(ORDERED))],
            vec![(
                proving_ground.url(STATIC_HOST, ORDER_PATH),
                json!("succeeded"),
                Value::Null
            )],
        )
    );
}

/// Under `auto`, the same crawl request, a sample, is sent as that `POST`
/// on `http`, from the browser by default or from the Run's transport
/// under `fetch.http_from: transport`, where the handler's script call
/// escalates it, and again as that `POST` by the browser it escalates to,
/// where the page names the ordered product; Provenance records the
/// browser run with the call that escalated it (issue 2471).
///
/// Covers: story:6-212, story:6-257, story:6-259, setting:fetch.http_from, operation:page.evaluate, error:attempt_ended
#[rstest::rstest]
#[case::from_the_browser(None, "browser_fetch")]
#[case::from_the_transport(Some("transport"), "http")]
fn a_post_crawl_request_auto_escalates_to_the_browser_reaches_the_site_as_that_post_on_both_tiers(
    #[case] http_from: Option<&str>,
    #[case] first_tier: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut fetch = json!({ "tier": "auto" });
    if let Some(http_from) = http_from {
        fetch["http_from"] = json!(http_from);
    }
    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-post-auto",
        &[order(&proving_ground)],
        &json!({ "order": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": fetch,
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut ordering(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-post-auto");
    assert!(engine.finished());

    assert_eq!(
        (
            order_methods(&proving_ground),
            seen,
            ended,
            tiers(&env, "tiers-post-auto"),
        ),
        (
            vec!["POST".to_owned(), "POST".to_owned()],
            vec![
                (json!("http"), json!("attempt_ended")),
                (json!("browser"), json!(ORDERED)),
            ],
            vec![(
                proving_ground.url(STATIC_HOST, ORDER_PATH),
                json!("succeeded"),
                Value::Null
            )],
            vec![
                json!(["order", first_tier, "sample", null, null]),
                json!(["order", "browser", "sample", null, "page.evaluate"]),
            ],
        )
    );
}

/// The `tiers` family's host: static listings at `/products/<n>`,
/// script-rendered ones at `/blog/<n>`, and `/mixed/<n>` one or the other
/// by the parity of `n`.
const TIERS_HOST: &str = "tiers.test";

/// The path of `url`, an absolute URL.
fn path_of(url: &str) -> String {
    format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default())
}

/// The handler that counts: it waits for at least one entry and pushes
/// one Record with its URL and the count it waited for, so its runs on
/// two tiers match where both see the same entries; a wait that fails ends
/// the attempt. Each request it handles is kept as its path and tier.
fn counting(seen: &mut Vec<(String, Value)>) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        let page_id = handle["page_id"].clone();
        let waited = inside.call(
            "locator.wait_for_count",
            &json!({
                "locator": { "page_id": page_id, "selector": ITEMS },
                "count": { "min": 1 },
            }),
        );
        let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
        seen.push((path_of(&url), handle["tier"].clone()));
        match waited.get("error") {
            Some(error) => json!({ "error": error }),
            None => json!({ "result": { "records": [{
                "dataset": "pages",
                "data": { "url": url, "count": waited["result"]["count"] },
            }] } }),
        }
    }
}

/// Seeds on [`TIERS_HOST`], each path with its Label.
fn tier_seeds(proving_ground: &ProvingGround, seeds: &[(&str, &str)]) -> Vec<Value> {
    seeds
        .iter()
        .map(|(path, label)| json!({ "url": proving_ground.url(TIERS_HOST, path), "label": label }))
        .collect()
}

/// Each `fetch_tier` Provenance entry of `run_id`, as `<label> <tier>
/// <chosen>`, with the call that escalated it when one did.
fn chosen(env: &TestEnv, run_id: &str) -> Vec<String> {
    tiers(env, run_id)
        .iter()
        .map(|entry| {
            let mut line = format!(
                "{} {} {}",
                entry[0].as_str().unwrap(),
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            );
            if let Some(call) = entry[4].as_str() {
                line = format!("{line} at {call}");
            }
            line
        })
        .collect()
}

/// `(path, tier)` for each handler request, from `(path, [tier, …])`.
fn runs(expected: &[(&str, &[&str])]) -> Vec<(String, Value)> {
    expected
        .iter()
        .flat_map(|(path, tiers)| tiers.iter().map(|tier| ((*path).to_owned(), json!(tier))))
        .collect()
}

/// Under `fetch.tier: auto`, a Label of static pages is learned onto
/// `http` and a Label of script-rendered pages onto `browser`, each on one
/// domain by sample comparison: each Label's first three requests run on
/// `http` and then in the browser, the static ones pushing the same Record
/// on both, the script-rendered ones failing their wait on `http`; the
/// next static page is fetched over HTTP alone, the next script-rendered
/// one in the browser alone, and a script-rendered page of the learned
/// static Label still escalates at the call `http` cannot serve. Every
/// request succeeds, and Provenance names each run's tier and how it was
/// chosen: a sample, the comparison's choice, or an escalation.
///
/// Covers: story:6-257, story:5-115, operation:locator.wait_for_count, error:attempt_ended
#[test]
fn a_label_of_static_pages_is_learned_onto_http_and_one_of_scripted_pages_onto_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "tiers-learned",
        &tier_seeds(
            &proving_ground,
            &[
                ("/products/1", "static"),
                ("/products/2", "static"),
                ("/products/3", "static"),
                ("/products/4", "static"),
                ("/mixed/2", "static"),
                ("/blog/1", "scripted"),
                ("/blog/2", "scripted"),
                ("/blog/3", "scripted"),
                ("/blog/4", "scripted"),
            ],
        ),
        &json!({ "static": {}, "scripted": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut counting(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-learned");
    assert!(engine.finished());

    let both: &[&str] = &["http", "browser"];
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            ended
                .iter()
                .filter(|(_, kind, _)| *kind == "succeeded")
                .count(),
            chosen(&env, "tiers-learned"),
        ),
        (
            &json!("finished"),
            runs(&[
                ("/products/1", both),
                ("/products/2", both),
                ("/products/3", both),
                ("/products/4", &["http"]),
                ("/mixed/2", both),
                ("/blog/1", both),
                ("/blog/2", both),
                ("/blog/3", both),
                ("/blog/4", &["browser"]),
            ]),
            9,
            [
                "static browser_fetch sample",
                "static browser sample",
                "static browser_fetch sample",
                "static browser sample",
                "static browser_fetch sample",
                "static browser sample",
                "static browser_fetch compared",
                "static browser_fetch compared",
                "static browser escalation at locator.wait_for_count",
                "scripted browser_fetch sample",
                "scripted browser sample at locator.wait_for_count",
                "scripted browser_fetch sample",
                "scripted browser sample at locator.wait_for_count",
                "scripted browser_fetch sample",
                "scripted browser sample at locator.wait_for_count",
                "scripted browser compared",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    );
}

/// A sample comparison compares the Records each run pushed through
/// `crawl_request.push` too (ADR 0103): static pages whose handler pushes,
/// on each tier, a Record naming that tier, and answers none, disagree on
/// every sample, so the Label goes on `browser`, not `http`.
///
/// Covers: story:6-257, story:6-267, operation:crawl_request.push
#[test]
fn a_sample_compares_the_records_each_run_pushed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut seen = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "tiers-pushed",
        &tier_seeds(
            &proving_ground,
            &[
                ("/products/1", "static"),
                ("/products/2", "static"),
                ("/products/3", "static"),
                ("/products/4", "static"),
            ],
        ),
        &json!({ "static": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
            seen.push((path_of(&url), handle["tier"].clone()));
            inside.call(
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "pages",
                    "records": [{ "url": url, "tier": handle["tier"] }],
                }),
            );
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let both: &[&str] = &["http", "browser"];
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            seen,
            chosen(&env, "tiers-pushed"),
        ),
        (
            &json!("finished"),
            runs(&[
                ("/products/1", both),
                ("/products/2", both),
                ("/products/3", both),
                ("/products/4", &["browser"]),
            ]),
            [
                "static browser_fetch sample",
                "static browser sample",
                "static browser_fetch sample",
                "static browser sample",
                "static browser_fetch sample",
                "static browser sample",
                "static browser compared",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    );
}

/// One Label whose `/products` and `/blog` pages disagree is split by the
/// first segment of their paths: once its three samples disagree by
/// segment, `/blog` stays on `browser` and `/products` goes on `http` once
/// its own third sample matched; a segment first seen after the split,
/// `/mixed`, is sampled on its own, and having disagreed stays on
/// `browser`.
///
/// Covers: story:6-258, story:6-257
#[test]
fn a_label_whose_segments_disagree_is_split_by_first_segment_and_a_segment_that_disagrees_stays_on_browser()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        "tiers-split",
        &tier_seeds(
            &proving_ground,
            &[
                ("/products/1", "page"),
                ("/blog/1", "page"),
                ("/products/2", "page"),
                ("/products/3", "page"),
                ("/blog/2", "page"),
                ("/products/4", "page"),
                ("/mixed/1", "page"),
                ("/mixed/2", "page"),
                ("/mixed/3", "page"),
            ],
        ),
        &json!({ "page": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut counting(&mut seen),
    );
    let ended = outcomes(&mut engine, "tiers-split");
    assert!(engine.finished());

    let both: &[&str] = &["http", "browser"];
    assert_eq!(
        (
            seen,
            ended
                .iter()
                .filter(|(_, kind, _)| *kind == "succeeded")
                .count(),
            chosen(&env, "tiers-split"),
        ),
        (
            runs(&[
                ("/products/1", both),
                ("/blog/1", both),
                ("/products/2", both),
                ("/products/3", both),
                ("/blog/2", &["browser"]),
                ("/products/4", &["http"]),
                ("/mixed/1", both),
                ("/mixed/2", both),
                ("/mixed/3", &["browser"]),
            ]),
            9,
            [
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch sample",
                "page browser sample at locator.wait_for_count",
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch sample",
                "page browser sample",
                "page browser compared",
                "page browser_fetch compared",
                "page browser_fetch sample",
                "page browser sample",
                "page browser_fetch sample",
                "page browser sample at locator.wait_for_count",
                "page browser compared",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    );
}

/// A crawl request's answer that pushes one Record naming its URL, the
/// same on every tier.
fn same_everywhere(params: &Value) -> Value {
    json!({ "result": { "records": [{
        "dataset": "pages",
        "data": { "url": params["crawl_request"]["url"] },
    }] } })
}

/// What a Label learned survives a resume: a crawl whose static Label was
/// learned onto `http` by its three samples is killed while its fourth
/// request is handled, and the resumed crawl takes that request and the
/// next over HTTP alone, sampling nothing again; Provenance names each as
/// the comparison's choice.
///
/// Covers: story:6-257, operation:run.resume
#[test]
fn the_tier_a_label_learned_survives_a_resume() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = tier_seeds(
        &proving_ground,
        &[
            ("/products/1", "static"),
            ("/products/2", "static"),
            ("/products/3", "static"),
            ("/products/4", "static"),
            ("/products/5", "static"),
        ],
    );
    let mut settings = settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } }));
    settings["crawl"]["handler"] = json!({ "concurrency": 1 });
    // At `full`, each sample's outcome is in the file before the next
    // request: at `batch` the kill takes the open batch, and the resumed
    // crawl samples its requests again (ADR 0142).
    settings["run_state"] = json!({ "durability": "full" });
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut killed = engine_on(&env, &cache);
    killed.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "tiers-kept", "seeds": seeds, "labels": { "static": {} }, "settings": settings },
    }));
    let mut before = Vec::new();
    while before.len() < 7 {
        let message = killed.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            let params = &message["params"];
            before.push((
                path_of(params["crawl_request"]["url"].as_str().unwrap()),
                params["tier"].clone(),
            ));
            if before.len() < 7 {
                let mut reply = same_everywhere(params);
                reply["jsonrpc"] = json!("2.0");
                reply["id"] = message["id"].clone();
                killed.send(&reply);
            }
        }
    }
    killed.kill();
    let mut resuming = engine_on(&env, &cache);

    let mut after = Vec::new();
    let mut answer = |params: &Value| {
        after.push((
            path_of(params["crawl_request"]["url"].as_str().unwrap()),
            params["tier"].clone(),
        ));
        Some(same_everywhere(params))
    };
    let resumed = resuming.call(
        "resume",
        "run.resume",
        &json!({ "run_id": "tiers-kept", "settings": settings }),
        &mut answer,
    );
    let finished = resuming.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "tiers-kept" }),
        &mut answer,
    );
    assert!(resuming.finished());

    let both: &[&str] = &["http", "browser"];
    let provenance = chosen(&env, "tiers-kept");
    assert_eq!(
        (
            resumed.get("error"),
            &finished["result"]["lifecycle"],
            before,
            after,
            provenance
                .iter()
                .filter(|line| line.ends_with("sample"))
                .count(),
            provenance
                .iter()
                .filter(|line| !line.ends_with("sample"))
                .collect::<std::collections::BTreeSet<_>>(),
        ),
        (
            None,
            &json!("finished"),
            runs(&[
                ("/products/1", both),
                ("/products/2", both),
                ("/products/3", both),
                ("/products/4", &["http"]),
            ]),
            runs(&[("/products/4", &["http"]), ("/products/5", &["http"])]),
            6,
            std::collections::BTreeSet::from([&"static browser_fetch compared".to_owned()]),
        )
    );
}

/// A script-written app shell: its HTML holds an empty `#app`, which its
/// inline script fills before the page loads.
const SHELL_PATH: &str = "/shell";

/// A Label's declaration whose `session.page_check` expects an element the
/// CSS selector `selector` finds, its other settings `more`.
pub(crate) fn expecting(selector: &str, more: &Value) -> Value {
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
pub(crate) fn recording(
    seen: &mut Vec<(String, Value)>,
) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
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
/// writes, is escalated to the browser instead of failing: its sample's
/// handler is given the browser's page only, where the expected page
/// passes, and it ends `succeeded`; a static listing whose document meets
/// its Label's expected page is handled on HTTP, then in the browser as
/// its sample's second run. Provenance records the shell's browser run as
/// caused by `session.page_check`.
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
    assert!(engine.finished());

    seen.sort_by(|left, right| left.0.cmp(&right.0));
    let mut chosen = tiers(&env, "expected-auto");
    chosen.sort_by_key(Value::to_string);
    assert_eq!(
        (&report["result"]["lifecycle"], seen, ended, chosen),
        (
            &json!("finished"),
            vec![
                (listing.clone(), json!("http")),
                (listing.clone(), json!("browser")),
                (shell.clone(), json!("browser")),
            ],
            vec![
                (listing, json!("succeeded"), Value::Null),
                (shell, json!("succeeded"), Value::Null),
            ],
            vec![
                json!(["app", "browser", "sample", null, "session.page_check"]),
                json!(["app", "browser_fetch", "sample", null, null]),
                json!(["list", "browser", "sample", null, null]),
                json!(["list", "browser_fetch", "sample", null, null]),
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
    assert!(engine.finished());

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
