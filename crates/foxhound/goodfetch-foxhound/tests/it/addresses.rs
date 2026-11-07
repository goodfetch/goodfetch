//! The address check of a crawl request on the HTTP tier (spec #565's story
//! 98; ADR 0017), on Foxhound's executable against the Proving ground: a
//! seed whose name the resolver answers with an address the Run's egress
//! policy refuses ends `skipped` with `domain_policy` before anything is
//! sent, its robots.txt never read, and the Run report counts it with the
//! other skips; a seed whose name the `network.resolve` mapping gives the
//! Proving ground's address, the positive control, is fetched.
//!
//! `localhost` is the name: every OS's resolver answers it with a loopback
//! address, which the Proving ground listens on, so a connection that got
//! through would show in its record. A handler's own navigations and
//! requests on a served page meet the same check first, and a sitemap seed
//! does too.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::crawl_with;

/// The Proving ground's seed.
const SEED: u64 = 678;

/// The requests the Proving ground received for a host starting with
/// `host`.
fn requests_to(observations: &Value, host: &str) -> usize {
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|seen| seen.starts_with(host))
        })
        .count()
}

/// A redirect to a name that resolves to a private address ends its
/// request `skipped` with `domain_policy` before the gate reads that
/// host's robots.txt, nothing sent to it; a redirect on the site, the
/// positive control, is followed.
///
/// Covers: story:4-98, story:4-103, setting:fetch.tier
#[test]
fn a_redirect_to_a_name_that_resolves_to_a_private_address_ends_skipped_with_domain_policy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let refused = proving_ground.url("fetch-edge.test", "/redirect/address/name");
    let control = proving_ground.url("fetch-edge.test", "/redirect/302");
    let settings = json!({
        "fetch": { "tier": "http" },
        "crawl": { "scope": { "strategy": "all" } },
        "network": { "resolve": { "fetch-edge.test": [proving_ground.address().ip().to_string()] } },
        "egress": { "private_ranges": ["fetch-edge.test"] },
    });
    let mut engine = attached(&env);

    let mut handled = Vec::new();
    crawl_with(
        &mut engine,
        "redirected",
        &[
            json!({ "url": refused, "label": "page" }),
            json!({ "url": control, "label": "page" }),
        ],
        &settings,
        &mut |_, handle| {
            handled.push(handle["crawl_request"]["url"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "redirected" }),
        &mut |_| None,
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let mut outcomes: Vec<(Value, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].clone(),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by_key(|outcome| outcome.0.to_string());
    assert_eq!(
        (outcomes, handled, requests_to(&observations, "localhost")),
        (
            vec![
                (json!(control), json!("succeeded"), Value::Null),
                (json!(refused), json!("skipped"), json!("domain_policy")),
            ],
            vec![json!(control)],
            0,
        ),
        "{read}"
    );
}

/// On the browser tier too, a seed whose name resolves to an address the
/// policy refuses ends `skipped` with `domain_policy` before the browser
/// opens anything, robots.txt obeyed as by default: nothing reaches the
/// address, and the positive control is fetched in the browser.
///
/// Covers: story:4-98, setting:fetch.tier
#[test]
fn a_browser_tier_seed_whose_name_resolves_to_a_private_address_ends_skipped_unsent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();
    let refused = format!("http://localhost:{port}/catalog/1");
    let control = proving_ground.url("scope.test", "/");
    let settings = json!({
        "fetch": { "tier": "browser" },
        "network": { "resolve": { "scope.test": [proving_ground.address().ip().to_string()] } },
        "egress": { "private_ranges": ["scope.test"] },
    });
    let mut engine = crate::fetch_tiers::engine(&env);

    let mut handled = Vec::new();
    crawl_with(
        &mut engine,
        "addressed-browser",
        &[
            json!({ "url": refused, "label": "page" }),
            json!({ "url": control, "label": "page" }),
        ],
        &settings,
        &mut |_, handle| {
            handled.push(handle["crawl_request"]["url"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "addressed-browser" }),
        &mut |_| None,
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let mut outcomes: Vec<(Value, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].clone(),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by_key(|outcome| outcome.0.to_string());
    assert_eq!(
        (outcomes, handled, requests_to(&observations, "localhost")),
        (
            vec![
                (json!(refused), json!("skipped"), json!("domain_policy")),
                (json!(control), json!("succeeded"), Value::Null),
            ],
            vec![json!(control)],
            0,
        ),
        "{read}"
    );
}

/// Covers: story:4-98, setting:fetch.tier
#[test]
fn a_seed_whose_name_resolves_to_a_private_address_ends_skipped_with_domain_policy_unsent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();
    let refused = format!("http://localhost:{port}/catalog/1");
    let control = proving_ground.url("scope.test", "/");
    let settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { "scope.test": [proving_ground.address().ip().to_string()] } },
        "egress": { "private_ranges": ["scope.test"] },
    });
    let mut engine = attached(&env);

    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        "addressed",
        &[
            json!({ "url": refused, "label": "page" }),
            json!({ "url": control, "label": "page" }),
        ],
        &settings,
        &mut |_, handle| {
            handled.push(handle["crawl_request"]["url"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "addressed" }),
        &mut |_| None,
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let mut outcomes: Vec<(Value, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].clone(),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by_key(|outcome| outcome.0.to_string());
    let requests = ReceivedRequests::of(&observations);
    let to_localhost = requests
        .every()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|host| host.starts_with("localhost"))
        })
        .count();
    // The observations' own read is the one connection with no request.
    let connections = observations["connections"].as_array().unwrap().len() - 1;
    let skips = &report["result"]["requests"]["by_skip_reason"];
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            outcomes,
            handled,
            to_localhost,
            connections <= requests.every().count(),
            skips
                .as_array()
                .unwrap_or_else(|| panic!("{report}"))
                .contains(&json!({ "name": "domain_policy", "count": 1 })),
        ),
        (
            &json!("finished"),
            vec![
                (json!(refused), json!("skipped"), json!("domain_policy")),
                (json!(control), json!("succeeded"), Value::Null),
            ],
            vec![json!(control)],
            0,
            true,
            true,
        ),
        "{report}"
    );
}

/// What a handler's calls on its served page answered, its request's own
/// document `scope.test`'s on the `http` tier, robots.txt respected as by
/// default: each call, made for the refused origin `localhost`, answered
/// its Error code and its details' URL, or its status; and the requests
/// `localhost` received.
fn served_calls(calls: &dyn Fn(&str) -> Vec<(&'static str, Value)>) -> (Vec<Value>, usize) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();
    let calls = calls(&format!("http://localhost:{port}"));
    let settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { "scope.test": [proving_ground.address().ip().to_string()] } },
        "egress": { "private_ranges": ["scope.test"] },
    });
    let mut engine = attached(&env);

    let mut answers = Vec::new();
    let report = crawl_with(
        &mut engine,
        "served",
        &[json!({ "url": proving_ground.url("scope.test", "/"), "label": "page" })],
        &settings,
        &mut |inside, handle| {
            for (method, params) in &calls {
                let mut params = params.clone();
                params["page_id"] = handle["page_id"].clone();
                let answer = inside.call(method, &params);
                answers.push(answer.get("error").map_or_else(
                    || answer["result"]["status"].clone(),
                    |error| json!([error["data"]["code"], error["data"]["url"]]),
                ));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    (answers, requests_to(&observations, "localhost"))
}

/// A handler's own `goto` and `page.request` on a served page to an
/// address the Run's egress policy refuses answer `egress_refused` naming
/// the URL, as Collie's page does, before anything is sent for it, its
/// robots.txt included, so the code never depends on whether robots.txt is
/// respected (spec #565; ADR 0093).
///
/// Covers: story:4-98, error:egress_refused, operation:page.goto, operation:page.request
#[test]
fn a_served_pages_navigation_and_request_to_a_refused_address_answer_egress_refused() {
    let (answers, to_localhost) = served_calls(&|refused| {
        let url = format!("{refused}/catalog/1");
        vec![
            ("page.goto", json!({ "url": url })),
            ("page.request", json!({ "url": url })),
        ]
    });

    let named = |answer: &Value| {
        answer[1]
            .as_str()
            .is_some_and(|url| url.ends_with("/catalog/1"))
    };
    assert_eq!(
        (
            answers
                .iter()
                .map(|answer| (answer[0].clone(), named(answer)))
                .collect::<Vec<_>>(),
            to_localhost
        ),
        (
            vec![
                (json!("egress_refused"), true),
                (json!("egress_refused"), true)
            ],
            0
        ),
        "{answers:?}"
    );
}

/// A `page.request` from a served page whose header value holds a CR and
/// an LF is refused with `header_value_invalid` before anything is sent,
/// as Collie's is (ADR 0067; TESTING.md's hostile input).
///
/// Covers: error:header_value_invalid, operation:page.request
#[test]
fn a_served_pages_request_with_a_hostile_header_value_is_refused_unsent() {
    let (answers, to_localhost) = served_calls(&|refused| {
        vec![(
            "page.request",
            json!({
                "url": format!("{refused}/catalog/1"),
                "headers": { "x-note": "a\r\nx-injected: 1" },
            }),
        )]
    });

    assert_eq!(
        (answers[0][0].clone(), to_localhost),
        (json!("header_value_invalid"), 0),
        "{answers:?}"
    );
}

/// A sitemap seed whose name resolves to an address the policy refuses
/// ends `skipped` with `domain_policy` before anything is sent for it, its
/// robots.txt included, never retried as an unreachable robots.txt would
/// be (spec #565; #678's first criterion): the Run report's sitemap counts
/// name the refusal.
///
/// Covers: story:4-98
#[test]
fn a_sitemap_seed_whose_name_resolves_to_a_private_address_ends_skipped_unsent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();
    let refused = format!("http://localhost:{port}/sitemap.xml");
    let settings = json!({
        "fetch": { "tier": "http" },
        "crawl": { "retry": { "max": 2 } },
    });
    let mut engine = attached(&env);

    let report = crawl_with(
        &mut engine,
        "sitemap",
        &[json!({ "sitemap": refused, "rules": [{ "pattern": "**", "label": "page" }] })],
        &settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let sitemaps = &report["result"]["sitemaps"];
    let failures: Vec<(Value, Value)> = sitemaps[0]["failures"]
        .as_array()
        .unwrap_or_else(|| panic!("{report}"))
        .iter()
        .map(|failure| (failure["url"].clone(), failure["code"].clone()))
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            failures,
            requests_to(&observations, "localhost")
        ),
        (
            &json!("finished"),
            vec![(json!(refused), json!("domain_policy"))],
            0
        ),
        "{sitemaps}"
    );
}
