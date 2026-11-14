//! A crawl's progress through the engine's public interface over the
//! in-memory network (spec #213's Seam 3), on the paused clock (ADR 0102):
//! why its due requests wait, one group per reason, each with its count,
//! the domains it holds with when each is next admitted, and the setting
//! behind the wait with its value and layer. Each test follows
//! `run.progress` as a program does, from the first value to the one that
//! shows the wait it sets up. What the handler does is scripted.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, Reply, crawl, dependencies};

fn seed(url: &str) -> Value {
    json!({ "url": url, "label": "page" })
}

/// Follows the progress of `run_id` from its first value to the first that
/// holds a wait group of `reason`, answering every handler request with
/// `handler`, and answers that group.
async fn wait_group(
    client: &mut Client,
    run_id: &str,
    reason: &str,
    handler: &mut impl FnMut(&Value) -> Reply,
) -> Value {
    let mut params = json!({ "run_id": run_id });
    loop {
        let progress = client.call("run.progress", params.clone(), handler).await;
        let progress = &progress["result"];
        if let Some(group) = progress["waiting"]
            .as_array()
            .and_then(|groups| groups.iter().find(|group| group["reason"] == reason))
        {
            return group.clone();
        }
        assert_eq!(progress["last"], false, "the crawl ended: {progress}");
        params["after"] = progress["position"].clone();
    }
}

/// Follows the progress of `run_id` from its first value to the first that
/// `shows`, answering every handler request with `handler`, and answers
/// that value.
async fn first_showing(
    client: &mut Client,
    run_id: &str,
    shows: impl Fn(&Value) -> bool,
    handler: &mut impl FnMut(&Value) -> Reply,
) -> Value {
    let mut params = json!({ "run_id": run_id });
    loop {
        let progress = client.call("run.progress", params.clone(), handler).await;
        let progress = progress["result"].clone();
        if shows(&progress) {
            return progress;
        }
        assert_eq!(progress["last"], false, "the crawl ended: {progress}");
        params["after"] = progress["position"].clone();
    }
}

/// A crawl's progress counts its requests by state, from its committed
/// state, lists each in flight with its phase, attempt and start, and
/// says how many handlers run against `crawl.handler.concurrency`.
///
/// Covers: story:3-162, operation:run.progress
#[test]
fn a_crawls_progress_counts_its_requests_and_lists_those_in_flight() {
    let env = TestEnv::new();
    let mut handler = |params: &Value| {
        if params["crawl_request"]["url"] == "https://a.test/1" {
            Reply::Silent
        } else {
            Reply::Records(Vec::new())
        }
    };

    let progress = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "counted",
                        "seeds": [seed("https://a.test/1"), seed("https://b.test/1")],
                    }),
                    &mut handler,
                )
                .await;
            first_showing(
                client,
                "counted",
                |progress| progress["requests"]["succeeded"] == 1,
                &mut handler,
            )
            .await
        },
    );

    assert_eq!(
        (
            &progress["requests"],
            &progress["in_flight"],
            &progress["concurrency"]["handlers"],
            &progress["concurrency"]["handler_concurrency"],
            &progress["idle"],
        ),
        (
            &json!({
                "pending": 0, "fetching": 0, "handling": 1, "in_human_pause": 0,
                "succeeded": 1, "failed": 0, "skipped": 0, "blocked": 0,
            }),
            &json!([{
                "request_id": "1",
                "url": "https://a.test/1",
                "phase": "handling",
                "tier": "http",
                "attempt": 1,
                "started_at": "2026-01-05T09:00:00Z",
            }]),
            &json!(1),
            &json!(4),
            &json!(false),
        )
    );
}

/// A subscription to the engine's events reads a crawl's requests in
/// flight in the `run.progress` events its Run publishes: the request in
/// its handler, as `run.progress` answers it.
///
/// Covers: story:3-162, event:run.progress, operation:events.subscribe
#[test]
fn a_subscription_reads_a_crawls_requests_in_flight_in_its_runs_progress_events() {
    const SUBSCRIPTION: i64 = 1000;
    let env = TestEnv::new();
    // The requests in flight of the first `run.progress` event of a batch
    // that shows one in its handler, or of the Run's last.
    let handling = |message: &Value| {
        let events = message["params"]["events"].as_array()?;
        events
            .iter()
            .filter(|event| event["event"] == "run.progress")
            .map(|event| &event["data"])
            .find(|progress| {
                progress["in_flight"][0]["phase"] == "handling" || progress["last"] == true
            })
            .map(|progress| progress["in_flight"].clone())
    };

    let in_flight = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .send(&json!({
                    "jsonrpc": "2.0", "id": SUBSCRIPTION, "method": "events.subscribe", "params": {},
                }))
                .await;
            client
                .send(&json!({
                    "jsonrpc": "2.0", "method": "$/credit",
                    "params": { "id": SUBSCRIPTION, "credits": 1000 },
                }))
                .await;
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "published", "seeds": [seed("https://a.test/1")] }),
                    &mut |_| Reply::Silent,
                )
                .await;
            // What came before the reply to `crawl.start`, then what comes.
            if let Some(found) = client
                .arrivals
                .iter()
                .find_map(|(message, _)| handling(message))
            {
                return found;
            }
            loop {
                let message = client
                    .next()
                    .await
                    .expect("the engine closed before the event");
                if let Some(found) = handling(&message) {
                    return found;
                }
            }
        },
    );

    assert_eq!(
        in_flight,
        json!([{
            "request_id": "1",
            "url": "https://a.test/1",
            "phase": "handling",
            "tier": "http",
            "attempt": 1,
            "started_at": "2026-01-05T09:00:00Z",
        }])
    );
}

/// A keep-alive crawl with no work says it is idle until a seed comes.
///
/// Covers: story:3-162, operation:run.progress, setting:crawl.keep_alive
#[test]
fn a_keep_alive_crawl_with_no_work_says_it_is_idle() {
    let env = TestEnv::new();
    let mut handler = |_: &Value| Reply::Records(Vec::new());

    let progress = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "kept",
                        "seeds": [seed("https://a.test/1")],
                        "settings": { "crawl": { "keep_alive": true } },
                    }),
                    &mut handler,
                )
                .await;
            first_showing(
                client,
                "kept",
                |progress| progress["idle"] == true,
                &mut handler,
            )
            .await
        },
    );

    assert_eq!(
        (&progress["requests"]["succeeded"], &progress["waiting"]),
        (&json!(1), &json!([]))
    );
}

/// A domain at its own concurrency holds its next request until one in
/// flight ends, so it is next admitted at no known time.
///
/// Covers: story:3-162, operation:run.progress, setting:politeness.domain.concurrency
#[test]
fn a_domain_at_its_concurrency_holds_its_due_requests_under_domain_concurrency() {
    let env = TestEnv::new();

    let group = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "held",
                        "seeds": [seed("https://shop.test/1"), seed("https://shop.test/2")],
                        "settings": { "politeness": { "domain": { "concurrency": 1 } } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            wait_group(client, "held", "domain_concurrency", &mut |_| Reply::Silent).await
        },
    );

    assert_eq!(
        group,
        json!({
            "reason": "domain_concurrency",
            "count": 1,
            "domains": [{ "domain": "shop.test" }],
            "setting": { "key": "politeness.domain.concurrency", "value": 1, "layer": "run" },
        })
    );
}

/// Starts `seeds` as `run_id` under `settings`, then follows its progress
/// to its first wait group of `reason`, the handler answering each request
/// as `handler` does.
fn first_group(
    seeds: &Value,
    settings: &Value,
    reason: &str,
    handler: impl FnMut(&Value) -> Reply + Clone,
) -> Value {
    let env = TestEnv::new();
    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut start_handler = handler.clone();
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "waits", "seeds": seeds, "settings": settings }),
                    &mut start_handler,
                )
                .await;
            let mut handler = handler;
            wait_group(client, "waits", reason, &mut handler).await
        },
    )
}

/// A domain whose last request started less than its delay ago holds the
/// next until the delay has passed since that start.
///
/// Covers: story:3-162, operation:run.progress, setting:politeness.domain.delay
#[test]
fn a_domain_within_its_delay_holds_its_next_request_under_domain_delay_until_it_passed() {
    let group = first_group(
        &json!([seed("https://shop.test/1"), seed("https://shop.test/2")]),
        &json!({ "politeness": { "domain": { "delay": 3_600_000 } } }),
        "domain_delay",
        |_| Reply::Records(Vec::new()),
    );

    assert_eq!(
        group,
        json!({
            "reason": "domain_delay",
            "count": 1,
            "domains": [{ "domain": "shop.test", "next_admitted_at": "2026-01-05T10:00:00Z" }],
            "setting": { "key": "politeness.domain.delay", "value": 3_600_000, "layer": "run" },
        })
    );
}

/// A domain that backs off holds its requests under `backoff`, here the
/// retry of the request whose site answered 503, naming
/// `politeness.backoff.max`, or `politeness.retry_after.max` when the
/// site's `Retry-After` is longer than any backoff (ADR 0118).
///
/// Covers: story:3-162, operation:run.progress, setting:politeness.backoff.max, setting:politeness.retry_after.max
#[rstest::rstest]
#[case::a_backoff(json!({ "status": 503 }), "politeness.backoff.max", 60_000)]
#[case::a_retry_after_past_any_backoff(
    json!({ "status": 503, "retry_after": "2026-01-05T09:10:00Z" }),
    "politeness.retry_after.max",
    900_000
)]
fn a_domain_that_backs_off_holds_its_requests_under_backoff_naming_the_setting_behind_the_wait(
    #[case] details: Value,
    #[case] setting: &str,
    #[case] value: u64,
) {
    let group = first_group(
        &json!([seed("https://shop.test/1"), seed("https://shop.test/2")]),
        &json!({}),
        "backoff",
        move |params: &Value| {
            if params["crawl_request"]["url"] == "https://shop.test/1" && params["attempt"] == 1 {
                Reply::FailsWith("response_unavailable", true, details.clone())
            } else {
                Reply::Records(Vec::new())
            }
        },
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
            &json!("shop.test"),
            true,
            &json!({ "key": setting, "value": value, "layer": "default" }),
        )
    );
}

/// A due request no domain holds waits while the handlers already run
/// `crawl.handler.concurrency` requests.
///
/// Covers: story:3-162, operation:run.progress, setting:crawl.handler.concurrency
#[test]
fn a_due_request_waits_under_handler_concurrency_while_the_handlers_are_busy() {
    let group = first_group(
        &json!([seed("https://a.test/1"), seed("https://b.test/1")]),
        &json!({ "crawl": { "handler": { "concurrency": 1 } } }),
        "handler_concurrency",
        |_| Reply::Silent,
    );

    assert_eq!(
        group,
        json!({
            "reason": "handler_concurrency",
            "count": 1,
            "domains": [{ "domain": "b.test" }],
            "setting": { "key": "crawl.handler.concurrency", "value": 1, "layer": "run" },
        })
    );
}
