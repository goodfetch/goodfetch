//! Why a crawl's due requests wait, on Foxhound's executable over stdio
//! against the Proving ground (spec #213's Seam 1; ADR 0102): a crawl held
//! by each waiting reason the engine builds reports it in `run.progress`,
//! one group per reason, with its count, the domains it holds with when
//! each is next admitted, and the setting behind the wait with its value
//! and layer. The `governor` reason has its own case in `governor.rs`. The
//! same reasons are checked on the paused clock in process (the crawl
//! crate's `start/tests/progress.rs`, Seam 3), where a next admission's
//! time is exact; the executable runs on the OS clock, so here a time is
//! checked as given.
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;

/// The Proving ground's seed.
const SEED: u64 = 1547;

/// The `statuses` family's host: articles, and every HTTP error status.
const STATUSES: &str = "statuses.test";

/// The `sitemap-seed` family's host, a second domain of articles.
const ARTICLES: &str = "sitemap-seed.test";

/// The `scope` family's host, whose `/quiet` page answers the status its
/// query asks for.
const SCOPE: &str = "scope.test";

/// The settings of a crawl of the three hosts on `proving_ground`, with
/// `more` merged in.
fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let ip = proving_ground.address().ip().to_string();
    let mut settings = json!({
        "network": { "resolve": { STATUSES: [&ip], ARTICLES: [&ip], SCOPE: [&ip] } },
        "egress": { "private_ranges": [STATUSES, ARTICLES, SCOPE] },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        for (key, value) in more {
            settings.insert(key.clone(), value.clone());
        }
    }
    settings
}

/// How the test's handler answers a crawl request.
#[derive(Clone, Copy)]
enum Handling {
    /// It holds the request, unanswered, until the case ends.
    Holds,
    /// It commits nothing and ends the attempt.
    Answers,
}

/// Starts the crawl `waits` of `seeds` under `more` settings and, once
/// `crawl.start` answered, so the Run exists, follows its
/// progress from its first value to the first that holds a wait group of
/// `reason`, each crawl request meanwhile handled as `handling` says;
/// answers that group, once every held request is answered and the
/// engine ended.
fn first_group(seeds: &[String], more: &Value, reason: &str, handling: Handling) -> Value {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);
    let seeds: Vec<Value> = seeds
        .iter()
        .map(|seed| {
            let (host, path) = seed.split_once('/').unwrap();
            json!({ "url": proving_ground.url(host, &format!("/{path}")), "label": "page" })
        })
        .collect();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "waits", "seeds": seeds, "settings": settings(&proving_ground, more) },
    }));
    let mut held = Vec::new();
    let mut params = json!({ "run_id": "waits" });
    let mut started = false;
    let mut asked = false;
    let group = loop {
        if started && !asked {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "progress", "method": "run.progress", "params": params,
            }));
            asked = true;
        }
        let message = engine.next().expect("the engine closed");
        if message["method"] == "crawl_request.handle" {
            match handling {
                Handling::Holds => held.push(message),
                Handling::Answers => engine.send(
                    &json!({ "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] } }),
                ),
            }
            continue;
        }
        if message["id"] == "start" {
            assert_eq!(message["result"]["run_id"], "waits", "{message}");
            started = true;
        }
        if message["id"] != "progress" {
            continue;
        }
        asked = false;
        let progress = &message["result"];
        if let Some(group) = progress["waiting"]
            .as_array()
            .and_then(|groups| groups.iter().find(|group| group["reason"] == reason))
        {
            break group.clone();
        }
        assert_eq!(progress["last"], false, "the crawl ended: {message}");
        params["after"] = progress["position"].clone();
    };
    for request in held {
        engine.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }));
    }
    assert!(engine.finish());
    group
}

/// A domain at its concurrency, its one request held by the handler, holds
/// its next due request under `domain_concurrency`, naming
/// `politeness.domain.concurrency` with its value and layer.
///
/// Covers: story:3-162, operation:run.progress, setting:politeness.domain.concurrency
#[test]
fn a_domain_at_its_concurrency_holds_its_due_requests_under_domain_concurrency() {
    let group = first_group(
        &[
            format!("{STATUSES}/articles/A-0001"),
            format!("{STATUSES}/articles/A-0002"),
        ],
        &json!({ "politeness": { "domain": { "concurrency": 1 } } }),
        "domain_concurrency",
        Handling::Holds,
    );

    assert_eq!(
        group,
        json!({
            "reason": "domain_concurrency",
            "count": 1,
            "domains": [{ "domain": STATUSES }],
            "setting": { "key": "politeness.domain.concurrency", "value": 1, "layer": "run" },
        })
    );
}

/// A domain whose last request started less than its delay ago holds the
/// next under `domain_delay`, saying when it is next admitted.
///
/// Covers: story:3-162, operation:run.progress, setting:politeness.domain.delay
#[test]
fn a_domain_within_its_delay_holds_its_next_request_under_domain_delay() {
    let group = first_group(
        &[
            format!("{STATUSES}/articles/A-0001"),
            format!("{STATUSES}/articles/A-0002"),
        ],
        &json!({ "politeness": { "domain": { "delay": 3_600_000 } } }),
        "domain_delay",
        Handling::Answers,
    );

    assert_eq!(
        (
            &group["count"],
            &group["domains"][0]["domain"],
            group["domains"][0]["next_admitted_at"].is_string(),
            &group["setting"],
        ),
        (
            &json!(1),
            &json!(STATUSES),
            true,
            &json!({ "key": "politeness.domain.delay", "value": 3_600_000, "layer": "run" }),
        )
    );
}

/// A domain whose site answered `503` backs off: its requests wait under
/// `backoff`, saying when it is next admitted and naming
/// `politeness.backoff.max` (ADR 0118). Each page is fetched on the `http`
/// tier before its handler runs, one at a time, the `503` first, and the
/// backoff's first wait outlasts the case.
///
/// Covers: story:3-162, operation:run.progress, setting:politeness.backoff.max
#[test]
fn a_domain_whose_site_answered_503_holds_its_requests_under_backoff() {
    let group = first_group(
        &[
            format!("{SCOPE}/quiet?mark=backoff&status=503"),
            format!("{SCOPE}/quiet?mark=next"),
        ],
        &json!({
            "fetch": { "tier": "http" },
            "politeness": {
                "domain": { "concurrency": 1, "delay": 0 },
                "backoff": { "initial": 30_000 },
            },
        }),
        "backoff",
        Handling::Answers,
    );

    assert_eq!(
        (
            &group["domains"][0]["domain"],
            group["domains"][0]["next_admitted_at"].is_string(),
            &group["setting"],
        ),
        (
            &json!(SCOPE),
            true,
            &json!({ "key": "politeness.backoff.max", "value": 60_000, "layer": "default" }),
        )
    );
}

/// A due request no domain holds waits under `handler_concurrency` while
/// the handlers already run `crawl.handler.concurrency` requests.
///
/// Covers: story:3-162, operation:run.progress, setting:crawl.handler.concurrency
#[test]
fn a_due_request_waits_under_handler_concurrency_while_the_handlers_are_busy() {
    let group = first_group(
        &[
            format!("{STATUSES}/articles/A-0001"),
            format!("{ARTICLES}/books/1"),
        ],
        &json!({ "crawl": { "handler": { "concurrency": 1 } } }),
        "handler_concurrency",
        Handling::Holds,
    );

    assert_eq!(
        (&group["reason"], &group["count"], &group["setting"]),
        (
            &json!("handler_concurrency"),
            &json!(1),
            &json!({ "key": "crawl.handler.concurrency", "value": 1, "layer": "run" }),
        )
    );
}
