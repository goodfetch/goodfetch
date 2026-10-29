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
//! through would show in its record.

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
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
    let mut engine = Attached::start(&env);

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
    assert!(engine.finish());

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
    assert!(engine.finish());

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
    let mut engine = Attached::start(&env);

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
    assert!(engine.finish());

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
