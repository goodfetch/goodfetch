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

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use goodfetch_test_support::{
    Answer, EgressRule, Family, Outcome, ProvingGround, ReceivedRequests, Refusal, Runner,
    SsrfCase, SsrfTable, TestEnv,
};
use serde_json::{Value, json};

use crate::backoff_classes::held_for;
use crate::frontier::crawl_with;

/// The Proving ground's seed.
const SEED: u64 = 678;

/// The requests the Proving ground received for a host starting with
/// `host`.
fn requests_to(observations: &Value, host: &str) -> usize {
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| request.host.starts_with(host))
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
    let mut engine = crate::site::engine_on(&env, &proving_ground);

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
    let mut engine = crate::site::engine_on(&env, &proving_ground);

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
        .filter(|request| request.host.starts_with("localhost"))
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
    let mut engine = crate::site::engine_on(&env, &proving_ground);

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
    let mut engine = crate::site::engine_on(&env, &proving_ground);

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

/// A crawl request skipped with `domain_policy` for a refused address does
/// not back its domain off: a following request to an allowed address on the
/// same domain key is admitted immediately with no backoff wait, and the
/// Politeness file holds no backoff on that domain key (moved from #678).
///
/// Covers: story:4-98, story:4-102, setting:egress.private_ranges, setting:politeness.backoff.initial
#[test]
fn a_domain_policy_refusal_does_not_back_its_domain_off() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip().to_string();
    let refused = proving_ground.url("refused.scope.test", "/catalog/1");
    let allowed = proving_ground.url("scope.test", "/");
    let politeness_file = env.home().join("politeness.sqlite");
    let settings = json!({
        "fetch": { "tier": "http" },
        "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
        "network": {
            "resolve": {
                "refused.scope.test": [&ip],
                "scope.test": [&ip],
            },
        },
        "egress": { "private_ranges": ["scope.test"] },
        "politeness": {
            "domain": { "concurrency": 1, "delay": 0 },
            "backoff": { "initial": 120_000 },
        },
        "site_preferences": { "respect_robots_txt": false },
    });
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        "no-backoff",
        &[
            json!({ "url": refused, "label": "page" }),
            json!({ "url": allowed, "label": "page" }),
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
        &json!({ "run_id": "no-backoff" }),
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
    let held = politeness_file.exists()
        && held_for(&politeness_file, "scope.test", Duration::from_secs(1));
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            outcomes,
            handled,
            requests_to(&observations, "refused.scope.test"),
            requests_to(&observations, "scope.test"),
            held,
        ),
        (
            &json!("finished"),
            vec![
                (json!(refused), json!("skipped"), json!("domain_policy")),
                (json!(allowed), json!("succeeded"), Value::Null),
            ],
            vec![json!(allowed)],
            0,
            1,
            false,
        ),
        "{report}\n{read}"
    );
}

/// A sitemap discovered through a site's `robots.txt` (`Sitemap:` line)
/// whose host resolves to a private address ends `skipped` with
/// `domain_policy`, unsent and never retried even when `crawl.retry.max`
/// allows retries (moved from #678).
///
/// Covers: story:4-98, story:4-102, setting:egress.private_ranges
#[test]
fn a_robots_txt_sitemap_whose_name_resolves_to_a_private_address_ends_skipped_unsent_and_unretried()
{
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip().to_string();
    let robots = proving_ground.url("sitemap-seed.test", "/robots.txt");
    let partner = proving_ground.url("sitemap-partner.test", "/cross.xml");
    let settings = json!({
        "fetch": { "tier": "http" },
        "crawl": { "retry": { "max": 2 } },
        "network": {
            "resolve": {
                "sitemap-seed.test": [&ip],
                "sitemap-partner.test": [&ip],
            },
        },
        "egress": { "private_ranges": ["sitemap-seed.test"] },
    });
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let report = crawl_with(
        &mut engine,
        "robots-sitemap-private",
        &[json!({
            "sitemap": robots,
            "rules": [{ "pattern": "**/books/*", "label": "book" }],
        })],
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
    let requests = ReceivedRequests::of(&observations);
    let connections = observations["connections"].as_array().unwrap().len() - 1;
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            sitemaps[0]["read"].clone(),
            failures,
            requests_to(&observations, "sitemap-seed.test"),
            requests_to(&observations, "sitemap-partner.test"),
            connections <= requests.every().count(),
        ),
        (
            &json!("finished"),
            json!(0),
            vec![(json!(partner), json!("domain_policy"))],
            1,
            0,
            true,
        ),
        "{report}"
    );
}

/// How many connections the Proving ground's listener `name` accepted.
fn listener_connections(proving_ground: &ProvingGround, name: &str) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|connection| connection["listener"] == json!(name))
        .count()
}

/// `case`'s URL pointed at `port` when it parses, or as written when the
/// WHATWG parser refuses it.
fn case_url_at(case: &SsrfCase, port: u16) -> String {
    match url::Url::parse(&case.url) {
        Ok(mut url) => {
            let _ = url.set_port(Some(port));
            url.to_string()
        }
        Err(_) => case.url.clone(),
    }
}

/// Positive controls for the Frontier SSRF runner: an in-scope seed with a
/// public stand-in address reaches the Proving ground's watched target.
const FRONTIER_CONTROLS: &str = r#"
[[cases]]
name = "allowed_seed_reaches_target"
family = "mixed_answers"
url = "http://allowed.ssrf.test/"
lookups = [{ name = "allowed.ssrf.test", answers = ["public"] }]
target = "public"
refusal = { code = "egress_refused", rule = "private_address" }

[[cases]]
name = "allowed_domain_reaches_target"
family = "domain_policy"
url = "http://scope.test/"
domains = { allowed = ["scope.test"] }
lookups = [{ name = "scope.test", answers = ["public"] }]
target = "public"
refusal = { code = "egress_refused", rule = "domain_policy" }
"#;

/// Builds the crawl settings for a batch of Frontier SSRF cases sharing one
/// domain policy.
fn frontier_group_settings(cases: &[&SsrfCase], public: IpAddr, is_control: bool) -> Value {
    let mut resolve = serde_json::Map::new();
    let mut private_ranges: Vec<String> = Vec::new();
    for case in cases {
        for lookup in &case.lookups {
            let answers: Vec<String> = lookup
                .answers
                .iter()
                .map(|answer| match answer {
                    Answer::Public => public.to_string(),
                    Answer::Address(address) => address.to_string(),
                })
                .collect();
            resolve.insert(lookup.name.clone(), json!(answers));
            if is_control && !private_ranges.contains(&lookup.name) {
                private_ranges.push(lookup.name.clone());
            }
        }
    }
    let (allowed, blocked) = cases[0].domains.as_ref().map_or_else(
        || (vec!["*".to_owned()], Vec::new()),
        |domains| (domains.allowed.clone(), domains.blocked.clone()),
    );
    json!({
        "fetch": { "tier": "http" },
        "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
        "network": { "resolve": resolve },
        "egress": {
            "allowed_domains": allowed,
            "blocked_domains": blocked,
            "private_ranges": private_ranges,
        },
        "site_preferences": { "respect_robots_txt": false },
    })
}

/// Runs a batch of SSRF cases that share one egress policy on Foxhound's
/// executable, submitting each as a seed and recording per-case refusals and
/// watched-target connections.
fn run_frontier_group(
    engine: &mut goodfetch_test_client::Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    cases: &[&SsrfCase],
    is_control: bool,
) -> BTreeMap<String, Outcome> {
    let target: std::net::SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let public = target.ip();
    let settings = frontier_group_settings(cases, public, is_control);
    let urls: Vec<(String, String)> = cases
        .iter()
        .map(|case| (case.name.clone(), case_url_at(case, target.port())))
        .collect();
    let seeds: Vec<Value> = urls
        .iter()
        .map(|(_, url)| json!({ "url": url, "label": "page" }))
        .collect();
    let before = listener_connections(proving_ground, "target");
    let report = crawl_with(
        engine,
        run_id,
        &seeds,
        &settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let seeds_answer = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let outcomes_answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let after = listener_connections(proving_ground, "target");
    let connected = vec![public; after - before];

    let mut seed_reasons: BTreeMap<String, String> = BTreeMap::new();
    for entry in seeds_answer["result"]["seeds"].as_array().unwrap() {
        if entry["kind"] == "not_added" {
            seed_reasons.insert(
                entry["url"].as_str().unwrap().to_owned(),
                entry["reason"].as_str().unwrap().to_owned(),
            );
        }
    }
    let mut skip_reasons: BTreeMap<String, String> = BTreeMap::new();
    for outcome in outcomes_answer["result"]["outcomes"].as_array().unwrap() {
        if outcome["kind"] == "skipped" {
            skip_reasons.insert(
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                outcome["reason"].as_str().unwrap().to_owned(),
            );
        }
    }

    let mut results = BTreeMap::new();
    for (case, (name, url)) in cases.iter().zip(&urls) {
        let refused_reason = seed_reasons.get(url).or_else(|| skip_reasons.get(url));
        let refusal = match refused_reason.map(String::as_str) {
            Some("domain_policy" | "invalid_url") => Some(Refusal::EgressRefused {
                rule: match case.family {
                    Family::DomainPolicy => EgressRule::DomainPolicy,
                    _ => EgressRule::PrivateAddress,
                },
            }),
            _ => None,
        };
        results.insert(
            name.clone(),
            Outcome {
                refusal,
                connections: connected.clone(),
            },
        );
    }
    results
}

/// Verifies that Frontier `crawl_request.enqueue` and sitemap admission refuse
/// private, encoded, mixed-answer, and domain-policy SSRF URLs with zero
/// connections to the Proving ground's watched target.
fn assert_frontier_enqueue_and_sitemap_refuse_ssrf(
    engine: &mut goodfetch_test_client::Attached,
    proving_ground: &ProvingGround,
    target: std::net::SocketAddr,
) {
    let public = target.ip();
    let port = target.port();
    let allowed_seed = format!("http://allowed.ssrf.test:{port}/");
    let enqueue_urls = [
        format!("http://127.0.0.1:{port}/from-enqueue"),
        format!("http://2130706433:{port}/from-enqueue"),
        format!("http://[::1]:{port}/from-enqueue"),
        format!("http://mixed.ssrf.test:{port}/from-enqueue"),
        format!("http://elsewhere.test:{port}/from-enqueue"),
    ];
    let settings = json!({
        "fetch": { "tier": "http" },
        "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
        "network": {
            "resolve": {
                "allowed.ssrf.test": [public.to_string()],
                "mixed.ssrf.test": [public.to_string(), "10.0.0.1"],
                "elsewhere.test": [public.to_string()],
            }
        },
        "egress": {
            "allowed_domains": ["allowed.ssrf.test", "mixed.ssrf.test"],
            "private_ranges": ["allowed.ssrf.test"],
        },
        "site_preferences": { "respect_robots_txt": false },
    });
    let before = listener_connections(proving_ground, "target");
    let mut enqueued_reasons = Vec::new();
    let report = crawl_with(
        engine,
        "frontier-enqueue-ssrf",
        &[json!({ "url": allowed_seed, "label": "page" })],
        &settings,
        &mut |inside, handle| {
            for url in &enqueue_urls {
                let answer = inside.enqueue(handle, url, "page", None);
                enqueued_reasons.push((
                    url.clone(),
                    answer["kind"].as_str().unwrap_or_default().to_owned(),
                    answer["reason"].as_str().unwrap_or_default().to_owned(),
                ));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let after = listener_connections(proving_ground, "target");
    // Only the single allowed seed reached the watched target; zero enqueued
    // SSRF targets opened a connection.
    assert_eq!(after - before, 1, "enqueued SSRF URLs must not connect");
    let outcomes = engine.call(
        "enqueue-outcomes",
        "run.outcomes",
        &json!({ "run_id": "frontier-enqueue-ssrf" }),
        &mut |_| None,
    );
    let skipped_urls: BTreeMap<String, String> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|outcome| outcome["kind"] == "skipped")
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                outcome["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for (url, kind, reason) in &enqueued_reasons {
        let refused_at_enqueue = kind == "not_added" && reason == "domain_policy";
        let skipped_before_fetch =
            skipped_urls.get(url).map(String::as_str) == Some("domain_policy");
        assert!(
            refused_at_enqueue || skipped_before_fetch,
            "enqueued SSRF URL {url} was neither refused at enqueue ({kind}/{reason}) nor skipped ({skipped_urls:?})"
        );
    }

    // Sitemap admission to a private target is also refused with zero connections.
    let before_sitemap = listener_connections(proving_ground, "target");
    let sitemap_report = crawl_with(
        engine,
        "frontier-sitemap-ssrf",
        &[json!({
            "sitemap": format!("http://127.0.0.1:{port}/sitemap.xml"),
            "rules": [{ "pattern": "**", "label": "page" }],
        })],
        &json!({
            "fetch": { "tier": "http" },
            "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
            "site_preferences": { "respect_robots_txt": false },
        }),
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(
        sitemap_report["result"]["lifecycle"], "finished",
        "{sitemap_report}"
    );
    let after_sitemap = listener_connections(proving_ground, "target");
    assert_eq!(
        after_sitemap - before_sitemap,
        0,
        "private sitemap must not connect"
    );
    assert_eq!(
        sitemap_report["result"]["sitemaps"][0]["failures"][0]["code"],
        json!("domain_policy"),
        "{sitemap_report}"
    );
}

/// Every case of the SSRF table Foxhound's Frontier meets (encodings, IPv6
/// forms, every non-global IANA block, mixed answers and domain policy) is
/// refused before any connection reaches the Proving ground's watched target
/// on Foxhound's executable, paired with positive controls that reach it
/// (ADR 0067, ADR 0094).
///
/// Covers: story:4-121, story:4-98
#[test]
fn every_case_of_the_ssrf_table_the_frontier_meets_is_refused_before_any_connection() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target: std::net::SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let public = target.ip();
    let runner = Runner::committed_foxhound(
        "frontier",
        file!(),
        "every_case_of_the_ssrf_table_the_frontier_meets_is_refused_before_any_connection",
    )
    .unwrap();
    let controls = SsrfTable::parse(FRONTIER_CONTROLS).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut outcomes: BTreeMap<String, Outcome> = BTreeMap::new();
    for (index, control) in controls.cases.iter().enumerate() {
        outcomes.extend(run_frontier_group(
            &mut engine,
            &proving_ground,
            &format!("frontier-control-{index}"),
            &[control],
            true,
        ));
    }

    // Group refused cases by their domain policy and host-resolution map so
    // non-colliding cases run in batches on the same Foxhound process.
    let mut groups: Vec<Vec<&SsrfCase>> = Vec::new();
    for case in runner.cases() {
        let same_policy = |group: &Vec<&SsrfCase>| {
            let same_domains = group[0].domains == case.domains;
            let same_lookups = case.lookups.iter().all(|lookup| {
                group.iter().all(|other| {
                    other
                        .lookups
                        .iter()
                        .all(|ol| ol.name != lookup.name || ol.answers == lookup.answers)
                })
            });
            let same_url = group
                .iter()
                .all(|other| case_url_at(other, target.port()) != case_url_at(case, target.port()));
            same_domains && same_lookups && same_url
        };
        if let Some(group) = groups.iter_mut().find(|group| same_policy(group)) {
            group.push(case);
        } else {
            groups.push(vec![case]);
        }
    }
    for (index, group) in groups.iter().enumerate() {
        outcomes.extend(run_frontier_group(
            &mut engine,
            &proving_ground,
            &format!("frontier-cases-{index}"),
            group,
            false,
        ));
    }
    assert_frontier_enqueue_and_sitemap_refuse_ssrf(&mut engine, &proving_ground, target);
    assert!(engine.finished());

    let result = runner.run(&controls.cases, public, |case| {
        outcomes.get(&case.name).cloned().unwrap()
    });
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// Positive controls for the HTTP tier redirect runner: a status redirect
/// and a `<meta http-equiv="refresh">` to an allowed page reach the target.
const REDIRECT_CONTROLS: &str = r#"
[[cases]]
name = "status_to_an_allowed_host"
family = "redirect"
url = "http://1.redirects.test:PORT/leaves/1"
target = "public"
refusal = { code = "egress_refused", rule = "domain_policy" }

[[cases]]
name = "meta_refresh_to_an_allowed_page"
family = "redirect"
url = "http://fetch-edge.test:PORT/refresh/meta/0"
target = "public"
refusal = { code = "egress_refused", rule = "scheme" }
"#;

/// Settings for the HTTP tier redirect SSRF runner.
fn redirect_runner_settings(ip: IpAddr, cases: &[SsrfCase]) -> Value {
    let mut resolve = serde_json::Map::new();
    for host in [
        "1.redirects.test",
        "redirects.test",
        "catalog.test",
        "fetch-edge.test",
    ] {
        resolve.insert(host.to_owned(), json!([ip.to_string()]));
    }
    for case in cases {
        for lookup in case.lookups.iter().skip(1) {
            let addresses: Vec<String> = lookup
                .answers
                .iter()
                .filter_map(|answer| match answer {
                    Answer::Address(address) => Some(address.to_string()),
                    Answer::Public => None,
                })
                .collect();
            resolve.insert(lookup.name.clone(), json!(addresses));
        }
    }
    json!({
        "fetch": { "tier": "http" },
        "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
        "network": { "resolve": resolve },
        "egress": {
            "private_ranges": [
                "1.redirects.test",
                "redirects.test",
                "catalog.test",
                "fetch-edge.test",
            ],
        },
        "site_preferences": { "respect_robots_txt": false },
    })
}

/// Runs one redirect SSRF case on `engine` against `proving_ground`.
fn run_redirect_case(
    engine: &mut goodfetch_test_client::Attached,
    proving_ground: &ProvingGround,
    settings: &Value,
    run_id: &str,
    case: &SsrfCase,
) -> Outcome {
    let ip = proving_ground.address().ip();
    let port = proving_ground.address().port();
    let url = if case.url.starts_with("http://redirect.ssrf.test/") {
        proving_ground.url("1.redirects.test", &format!("/{}", case.name))
    } else {
        case.url.replace("PORT", &port.to_string())
    };
    let parsed = url::Url::parse(&url).unwrap();
    let own_host = parsed.host_str().unwrap().to_owned();
    let own_path = parsed.path().to_owned();
    let before = proving_ground.received_requests().unwrap().every().count();
    let report = crawl_with(
        engine,
        run_id,
        &[json!({ "url": url, "label": "page" })],
        settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let outcome = &read["result"]["outcomes"][0];
    let connections: Vec<IpAddr> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .skip(before)
        .filter(|request| {
            let host = request.host.split(':').next().unwrap_or_default();
            !(host == own_host && request.target == own_path)
        })
        .map(|_| ip)
        .collect();
    let refusal = match outcome["kind"].as_str() {
        Some("skipped") if outcome["reason"] == "domain_policy" => Some(Refusal::EgressRefused {
            rule: EgressRule::PrivateAddress,
        }),
        Some("failed")
            if outcome["error"]["code"] == "response_not_ok"
                && outcome["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("not an HTTP(S) one")) =>
        {
            Some(Refusal::EgressRefused {
                rule: EgressRule::Scheme,
            })
        }
        _ => None,
    };
    Outcome {
        refusal,
        connections,
    }
}

/// Every redirect case of the SSRF table on Foxhound's HTTP tier
/// (`301`/`302`/`303`/`307`/`308`, `Refresh` header, `<meta refresh>`,
/// private names, encoded addresses, and non-HTTP schemes) is refused or
/// unfollowed on Foxhound's executable with zero connections to the target,
/// paired with positive controls that follow to an allowed page (ADR 0067,
/// ADR 0094).
///
/// Covers: story:4-121, story:4-103
#[test]
fn every_redirect_case_of_the_ssrf_table_is_refused_on_the_http_tier() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip();
    let runner = Runner::committed_foxhound(
        "http_tier",
        file!(),
        "every_redirect_case_of_the_ssrf_table_is_refused_on_the_http_tier",
    )
    .unwrap();
    let controls = SsrfTable::parse(REDIRECT_CONTROLS).unwrap();
    let settings = redirect_runner_settings(ip, runner.cases());
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut run_index = 0usize;
    let result = runner.run(&controls.cases, ip, |case| {
        run_index += 1;
        run_redirect_case(
            &mut engine,
            &proving_ground,
            &settings,
            &format!("redirect-{run_index}"),
            case,
        )
    });
    assert!(engine.finished());

    // On the HTTP tier, `<meta http-equiv="refresh">` to a non-HTTP scheme
    // and `<script>` redirects are not followed or executed at all, so they
    // keep the stub document and reach nothing.
    let unfollowed = |name: &str| (name.to_owned(), None, false);
    assert_eq!(
        result.map_err(|failed| {
            let let_through: Vec<_> = failed
                .let_through
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect();
            (failed.no_control, failed.held, let_through)
        }),
        Err((
            false,
            Vec::new(),
            vec![
                unfollowed("script_location"),
                unfollowed("meta_refresh_to_file"),
            ]
        ))
    );
}

/// End-to-end DNS rebinding on Foxhound's executable on Linux (ADR 0067,
/// ADR 0094): Foxhound runs inside a `DnsNamespace` pointed at the Proving
/// ground's DNS fixture; a name alternating between a listed loopback
/// stand-in (`127.0.0.2`) and `127.0.0.1` succeeds on its first crawl
/// request and ends `skipped` with `domain_policy` on the next, with zero
/// connections to the watched `127.0.0.1` listener.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_types,
    reason = "the test binds two loopback listeners outside the engine to observe connections directly (ADR 0060)"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs a tiny local stand-in server on a dedicated OS thread outside any async worker"
)]
mod linux {
    use std::io::ErrorKind;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    use goodfetch_test_support::{DnsNamespace, ProvingGround, TestEnv};
    use serde_json::{Value, json};

    use super::SEED;
    use crate::attached;
    use crate::frontier::crawl_with;

    const CHILD: &str = "addresses::linux::in_the_namespace_a_rebinding_crawl_request_is_refused";

    /// Covers: story:4-121, story:4-102
    #[test]
    fn on_linux_a_rebinding_crawl_request_is_refused_on_foxhound() {
        let env = TestEnv::new();
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
        let namespace = DnsNamespace::enter(&env, fixture).unwrap();

        let output = namespace
            .output(
                &env,
                std::env::current_exe().unwrap(),
                [CHILD, "--include-ignored"],
            )
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && !stdout.contains("running 0 tests"),
            "status={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
    }

    #[ignore = "run as a child by addresses::linux::on_linux_a_rebinding_crawl_request_is_refused_on_foxhound"]
    #[test]
    fn in_the_namespace_a_rebinding_crawl_request_is_refused() {
        let env = TestEnv::new();
        let public = Ipv4Addr::new(127, 0, 0, 2);
        let public_server = TcpListener::bind(SocketAddr::from((public, 0))).unwrap();
        let port = public_server.local_addr().unwrap().port();
        let watched = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).unwrap();
        watched.set_nonblocking(true).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let stop_server = Arc::clone(&stop);
        public_server.set_nonblocking(true).unwrap();
        let server_thread = thread::spawn(move || {
            use std::io::{Read as _, Write as _};
            while !stop_server.load(Ordering::Relaxed) {
                match public_server.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut buf = [0u8; 4096];
                        let _ = stream.read(&mut buf);
                        let body = "<html><body><h1>ok</h1></body></html>";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let mut engine = attached(&env);

        // Two public answers for the first seed's address check + connection,
        // then `127.0.0.1` for the second seed.
        let host = "127-0-0-2.127-0-0-2.127-0-0-1.127-0-0-1.alternating.dns.test";
        let first = format!("http://{host}:{port}/page/1");
        let second = format!("http://{host}:{port}/page/2");
        let settings = json!({
            "fetch": { "tier": "http" },
            "crawl": { "retry": { "max": 0 } },
            "egress": { "private_ranges": ["127.0.0.2"] },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "site_preferences": { "respect_robots_txt": false },
        });

        let mut handled = Vec::new();
        let report = crawl_with(
            &mut engine,
            "rebind-http",
            &[
                json!({ "url": first, "label": "page" }),
                json!({ "url": second, "label": "page" }),
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
            &json!({ "run_id": "rebind-http" }),
            &mut |_| None,
        );
        assert!(engine.finished());
        stop.store(true, Ordering::Relaxed);
        let _ = server_thread.join();

        let mut outcomes: Vec<(Value, Value, Value)> = read["result"]["outcomes"]
            .as_array()
            .unwrap()
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
            (
                &report["result"]["lifecycle"],
                outcomes,
                handled,
                watched.accept().map(|_| ()).unwrap_err().kind(),
            ),
            (
                &json!("finished"),
                vec![
                    (json!(first), json!("succeeded"), Value::Null),
                    (json!(second), json!("skipped"), json!("domain_policy")),
                ],
                vec![json!(first)],
                ErrorKind::WouldBlock,
            ),
            "{report}\n{read}"
        );
    }
}
