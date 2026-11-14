//! A crawl's progress through the engine's public interface over the
//! in-memory network (spec #213's Seam 3), on the paused clock (ADR 0102):
//! why its due requests wait, one group per reason, each with its count,
//! the domains it holds with when each is next admitted, and the setting
//! behind the wait with its value and layer. Each test follows
//! `run.progress` as a program does, from the first value to the one that
//! shows the wait it sets up. What the handler does is scripted.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, Reply, SITEMAP, crawl, dependencies, run_file};

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

/// A live crawl's progress counts its requests from its Run state, in
/// memory, and reads none of them from its run file (#2167): a count
/// written into the file behind the crawl's back shows in no value of the
/// live Run. Its last value, which no Run state gives any more, is read
/// from the file and shows it.
///
/// Covers: story:3-162, operation:run.progress
#[test]
fn a_live_crawls_progress_counts_its_requests_in_memory_and_its_last_value_reads_the_run_file() {
    let env = TestEnv::new();
    let mut handler = |_: &Value| Reply::Records(Vec::new());

    let (live, last) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "live",
                        "seeds": [seed("https://a.test/1")],
                        "settings": { "crawl": { "keep_alive": true } },
                    }),
                    &mut handler,
                )
                .await;
            let idle = |progress: &Value| progress["idle"] == true;
            first_showing(client, "live", idle, &mut handler).await;
            run_file(&env, "live")
                .execute(
                    "INSERT INTO request_counts (kind, name, count) VALUES ('failed', 'planted', 7)",
                    [],
                )
                .unwrap();
            let live = client
                .call("run.progress", json!({ "run_id": "live" }), &mut handler)
                .await;
            client
                .call("run.finish", json!({ "run_id": "live" }), &mut handler)
                .await;
            let last = |progress: &Value| progress["last"] == true;
            let last = first_showing(client, "live", last, &mut handler).await;
            (live["result"].clone(), last)
        },
    );

    assert_eq!(
        (
            &live["last"],
            &live["requests"]["succeeded"],
            &live["requests"]["failed"],
            &last["requests"]["succeeded"],
            &last["requests"]["failed"],
        ),
        (&json!(false), &json!(1), &json!(0), &json!(1), &json!(7))
    );
}

/// What the run file of `run_id` counts of its crawl's requests by state
/// and of its sitemap seeds, as a value of its progress shows them with
/// no request in flight.
fn committed(env: &TestEnv, run_id: &str) -> (Value, Value) {
    let connection = run_file(env, run_id);
    let (ended, _) = crate::report::ended(&connection).unwrap();
    let requests = RequestProgress {
        pending: ended.pending,
        fetching: 0,
        handling: 0,
        in_human_pause: 0,
        succeeded: ended.succeeded,
        failed: ended.failed,
        skipped: ended.skipped,
        blocked: ended.blocked,
        by_skip_reason: ended.by_skip_reason,
        by_error_code: ended.by_error_code,
        by_block_cause: ended.by_block_cause,
    };
    let sitemaps = goodfetch_frontier::sitemap::reports(&connection).unwrap();
    (json!(requests), json!(sitemaps))
}

/// A live crawl's Run state counts its requests by state, in memory, as
/// its run file counts them (#2167): through a sitemap seed's read, a
/// request that succeeds, one that fails, one skipped and one blocked, a
/// seed added to the running crawl and the re-admission of its failures,
/// each value of its progress read once the crawl is idle again shows the
/// counts its run file then holds.
///
/// Covers: story:3-162, operation:run.progress, operation:run.add_seed, operation:run.retry_failed
#[test]
fn a_live_crawls_counts_in_memory_are_its_run_files_through_every_ending() {
    let env = TestEnv::new();
    let readmitted = std::cell::Cell::new(false);
    let mut handler = |params: &Value| match params["crawl_request"]["url"].as_str() {
        Some("https://shop.test/page/2") if !readmitted.get() => {
            Reply::Fails("handler_failed", false)
        }
        Some("https://shop.test/page/3") => Reply::Skips("Out of stock."),
        Some("https://shop.test/blocked") => Reply::Blocked("rate_limit"),
        _ => Reply::Records(Vec::new()),
    };
    let idle_with = |succeeded: u64| {
        move |progress: &Value| {
            progress["idle"] == true && progress["requests"]["succeeded"] == succeeded
        }
    };
    let shown = |progress: Value| (progress["requests"].clone(), progress["sitemaps"].clone());

    let (live, file) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut live = Vec::new();
            let mut file = Vec::new();
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "walked",
                        "seeds": [
                            {
                                "sitemap": format!("https://shop.test{SITEMAP}"),
                                "rules": [{ "pattern": "**", "label": "page" }],
                            },
                            seed("https://shop.test/blocked"),
                        ],
                        "settings": { "crawl": { "keep_alive": true } },
                    }),
                    &mut handler,
                )
                .await;
            live.push(shown(
                first_showing(client, "walked", idle_with(1), &mut handler).await,
            ));
            file.push(committed(&env, "walked"));
            client
                .call(
                    "run.add_seed",
                    json!({ "run_id": "walked", "seed": seed("https://shop.test/late") }),
                    &mut handler,
                )
                .await;
            live.push(shown(
                first_showing(client, "walked", idle_with(2), &mut handler).await,
            ));
            file.push(committed(&env, "walked"));
            readmitted.set(true);
            client
                .call(
                    "run.retry_failed",
                    json!({ "run_id": "walked", "blocked": true }),
                    &mut handler,
                )
                .await;
            live.push(shown(
                first_showing(client, "walked", idle_with(3), &mut handler).await,
            ));
            file.push(committed(&env, "walked"));
            client
                .call("run.finish", json!({ "run_id": "walked" }), &mut handler)
                .await;
            (live, file)
        },
    );

    assert_eq!(live, file);
    assert_eq!(
        live.iter()
            .map(|(requests, _)| {
                ["pending", "succeeded", "failed", "skipped", "blocked"]
                    .map(|state| requests[state].as_u64().unwrap())
            })
            .collect::<Vec<_>>(),
        [[0, 1, 1, 1, 1], [0, 2, 1, 1, 1], [0, 3, 0, 1, 1]],
        "the walk went through each ending"
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
