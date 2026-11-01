//! A crawl's admission through the engine's public interface over the
//! in-memory network (spec #213's Seam 3), on the paused clock: the order
//! the per-domain queues give its work items, the rate cap's spacing and
//! the queues' journal in the Run's file; the backoff and the `Retry-After`
//! a handler met, which hold its domain in the Politeness the Run shares,
//! and a `Retry-After` past `politeness.retry_after.max`, which ends the
//! domain's requests `failed` unsent (ADR 0118).

use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, page_of, run_file};

fn seed(url: &str) -> Value {
    json!({ "url": url, "label": "page" })
}

/// Starts `seeds` under `settings`, finishes the crawl with every attempt
/// answered at once, and answers the URLs in the order their requests came,
/// each with the paused clock's time since the first.
fn handled(env: &TestEnv, seeds: &Value, settings: &Value) -> Vec<(String, Duration)> {
    crawl(
        || dependencies(env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "fair", "seeds": seeds, "settings": settings }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |params: &Value| Reply::Records(vec![json!({ "page": page_of(params) })]);
            client
                .call("run.finish", json!({ "run_id": "fair" }), &mut handler)
                .await;
            let first = client.arrivals.first().map(|(_, at)| *at);
            client
                .arrivals
                .iter()
                .filter(|(message, _)| message["method"] == "crawl_request.handle")
                .map(|(message, at)| {
                    (
                        page_of(&message["params"]),
                        *at - first.expect("a request came"),
                    )
                })
                .collect()
        },
    )
}

fn urls(handled: &[(String, Duration)]) -> Vec<&str> {
    handled.iter().map(|(url, _)| url.as_str()).collect()
}

/// Whether `wait` is a first wait of the backoff's: drawn between 2 s and
/// three times that (ADR 0118).
fn a_first_backoff(wait: Duration) -> bool {
    (Duration::from_secs(2)..=Duration::from_secs(6)).contains(&wait)
}

/// The URLs of the handler requests the engine sent, as they came, each
/// with the paused clock's time since the first.
fn sent(client: &super::Client) -> Vec<(String, Duration)> {
    let first = client.arrivals.first().map(|(_, at)| *at);
    client
        .arrivals
        .iter()
        .filter(|(message, _)| message["method"] == "crawl_request.handle")
        .map(|(message, at)| {
            (
                page_of(&message["params"]),
                *at - first.expect("a request came"),
            )
        })
        .collect()
}

/// Covers: story:3-47
#[test]
fn a_heavy_domain_listed_first_lets_the_small_ones_take_their_turns() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://www.heavy.test/2"),
        seed("https://heavy.test/3"),
        seed("https://heavy.test/4"),
        seed("https://a.test/1"),
        seed("https://b.test/1"),
    ]);

    let handled = handled(
        &env,
        &seeds,
        &json!({ "crawl": { "handler": { "concurrency": 1 } } }),
    );

    assert_eq!(
        urls(&handled),
        [
            "https://heavy.test/1",
            "https://a.test/1",
            "https://b.test/1",
            "https://www.heavy.test/2",
            "https://heavy.test/3",
            "https://heavy.test/4",
        ]
    );
}

/// Covers: story:3-49
#[test]
fn a_seeds_priority_orders_its_domain_and_never_another() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://heavy.test/2"),
        { "url": "https://heavy.test/urgent", "label": "page", "priority": 10 },
        seed("https://a.test/1"),
        { "url": "https://a.test/later", "label": "page", "priority": -1 },
        seed("https://a.test/2"),
    ]);

    let handled = handled(
        &env,
        &seeds,
        &json!({ "crawl": { "handler": { "concurrency": 1 } } }),
    );

    assert_eq!(
        urls(&handled),
        [
            "https://heavy.test/urgent",
            "https://a.test/1",
            "https://heavy.test/1",
            "https://a.test/2",
            "https://heavy.test/2",
            "https://a.test/later",
        ]
    );
}

/// Covers: story:3-51, setting:admission.max_requests_per_minute
#[test]
fn a_rate_cap_spaces_a_crawls_requests_whatever_its_concurrency() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://a.test/1"),
        seed("https://b.test/1"),
    ]);

    let handled = handled(
        &env,
        &seeds,
        &json!({ "admission": { "max_requests_per_minute": 30 } }),
    );

    assert_eq!(
        handled.iter().map(|(_, at)| *at).collect::<Vec<_>>(),
        [0, 2, 4].map(Duration::from_secs)
    );
}

/// Covers: story:3-50
#[test]
fn a_crawls_work_items_are_journaled_with_their_domain_and_its_drained_queues_released() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://www.heavy.test/1"),
        seed("https://alice.github.io/1"),
    ]);

    handled(&env, &seeds, &json!({}));

    let file = run_file(&env, "fair");
    let keys: Vec<(String, String)> = file
        .prepare("SELECT id, domain_key FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let domains: i64 = file
        .query_row("SELECT count(*) FROM domains", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (keys, domains),
        (
            vec![
                ("1".to_owned(), "heavy.test".to_owned()),
                ("2".to_owned(), "alice.github.io".to_owned()),
            ],
            0
        )
    );
}

/// A request whose response's first byte never came backs its domain off:
/// the domain holds no turn for 2 s while the others go, and its retry
/// waits them out (ADRs 0004, 0021 and 0086).
///
/// Covers: story:3-50
#[test]
fn a_domain_whose_first_byte_timed_out_backs_off_while_the_others_go() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://heavy.test/2"),
        seed("https://a.test/1"),
    ]);
    let mut timed_out = false;

    let handled = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "slow", "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 1 } } } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if page_of(params) == "https://heavy.test/1" && !timed_out {
                    timed_out = true;
                    return Reply::FailsWith(
                        "deadline_exceeded",
                        true,
                        json!({
                            "limit": { "key": "timeout.first_byte", "value": 30_000, "layer": "default" },
                            "elapsed": 30_000,
                            "waited_for": "first_byte",
                        }),
                    );
                }
                Reply::Records(vec![json!({ "page": page_of(params) })])
            };
            client
                .call("run.finish", json!({ "run_id": "slow" }), &mut handler)
                .await;
            let first = client.arrivals.first().map(|(_, at)| *at);
            client
                .arrivals
                .iter()
                .filter(|(message, _)| message["method"] == "crawl_request.handle")
                .map(|(message, at)| {
                    (
                        page_of(&message["params"]),
                        *at - first.expect("a request came"),
                    )
                })
                .collect::<Vec<_>>()
        },
    );

    assert_eq!(
        urls(&handled),
        [
            "https://heavy.test/1",
            "https://a.test/1",
            "https://heavy.test/1",
            "https://heavy.test/2"
        ]
    );
    assert_eq!(handled[1].1, Duration::ZERO);
    assert!(a_first_backoff(handled[2].1), "{handled:?}");
    assert_eq!(handled[3].1, handled[2].1);
}

/// A handler that met the site's 429, failing with `blocked` for a
/// `rate_limit`, backs its domain off in the Politeness every run of the
/// user shares: the domain's next request waits its first backoff, from
/// 2 s to 6 s, while the others go (ADRs 0021, 0053 and 0118).
///
/// Covers: story:3-80
#[test]
fn a_429_a_handler_met_backs_its_domain_off_while_the_others_go() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://heavy.test/2"),
        seed("https://a.test/1"),
    ]);

    let handled = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "refused", "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 1 } } } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if page_of(params) == "https://heavy.test/1" {
                    return Reply::Blocked("rate_limit");
                }
                Reply::Records(vec![json!({ "page": page_of(params) })])
            };
            client
                .call("run.finish", json!({ "run_id": "refused" }), &mut handler)
                .await;
            let first = client.arrivals.first().map(|(_, at)| *at);
            client
                .arrivals
                .iter()
                .filter(|(message, _)| message["method"] == "crawl_request.handle")
                .map(|(message, at)| {
                    (
                        page_of(&message["params"]),
                        *at - first.expect("a request came"),
                    )
                })
                .collect::<Vec<_>>()
        },
    );

    assert_eq!(
        urls(&handled),
        [
            "https://heavy.test/1",
            "https://a.test/1",
            "https://heavy.test/2"
        ]
    );
    assert_eq!(handled[1].1, Duration::ZERO);
    assert!(a_first_backoff(handled[2].1), "{handled:?}");
}

/// Crawls `seeds` one request at a time under `settings`, the handler
/// failing `heavy.test/1` once as `failure` says after `took` of the
/// paused clock, then finishes the crawl; answers the requests sent, the
/// Run report and what `run.failed_requests` lists.
fn crawled_with(
    env: &TestEnv,
    run_id: &str,
    (settings, took): (&Value, Duration),
    failure: &(&'static str, bool, Value),
) -> (Vec<(String, Duration)>, Value, Value) {
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://heavy.test/2"),
        seed("https://a.test/1"),
    ]);
    let mut settings = settings.clone();
    settings["crawl"] = json!({ "handler": { "concurrency": 1 } });
    let mut failed = false;
    crawl(
        || dependencies(env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": run_id, "seeds": seeds, "settings": settings }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if page_of(params) == "https://heavy.test/1" && !failed {
                    failed = true;
                    let (code, retryable, details) = failure.clone();
                    return Reply::After(
                        took,
                        Box::new(Reply::FailsWith(code, retryable, details)),
                    );
                }
                Reply::Records(vec![json!({ "page": page_of(params) })])
            };
            let report = client
                .call("run.finish", json!({ "run_id": run_id }), &mut handler)
                .await;
            let failed_requests = client
                .call(
                    "run.failed_requests",
                    json!({ "run_id": run_id }),
                    &mut |_| Reply::Silent,
                )
                .await;
            (sent(client), report, failed_requests)
        },
    )
}

/// A site's `Retry-After` a handler met with its 429 or its 503 holds the
/// domain until the time the site asked, 30 s, past any first backoff:
/// nothing more is sent to it before then, while the other domains go
/// (ADR 0118).
///
/// Covers: story:3-159
#[rstest::rstest]
#[case::too_many_requests(("blocked", false, json!({ "cause": "rate_limit", "retry_after": "2026-01-05T09:00:30Z" })))]
#[case::unavailable(("response_unavailable", true, json!({ "status": 503, "retry_after": "2026-01-05T09:00:30Z" })))]
fn a_retry_after_a_handler_met_holds_its_domain_until_the_time_the_site_asked(
    #[case] failure: (&'static str, bool, Value),
) {
    let env = TestEnv::new();

    let (sent, _, _) = crawled_with(&env, "asked", (&json!({}), Duration::ZERO), &failure);

    let to_heavy: Vec<Duration> = sent
        .iter()
        .filter(|(url, _)| url.starts_with("https://heavy.test/"))
        .map(|(_, at)| *at)
        .collect();
    assert_eq!(to_heavy[0], Duration::ZERO);
    assert!(
        to_heavy[1..]
            .iter()
            .all(|at| *at >= Duration::from_secs(30)),
        "{sent:?}"
    );
    assert!(
        sent.contains(&("https://a.test/1".to_owned(), Duration::ZERO)),
        "{sent:?}"
    );
}

/// A site that asks for an hour, past the 15 minutes of
/// `politeness.retry_after.max`, gets nothing more and is not waited for:
/// the request whose 503 asked ends `failed` with no retry spent, the
/// domain's other request ends `failed` unsent with a `null` status, both
/// with `response_unavailable`, the time the site asked and a `fix` naming
/// the setting, the other domains go, and the Run finishes `incomplete`
/// with both listed for `run.retry_failed` (ADR 0118).
///
/// Covers: story:3-160, setting:politeness.retry_after.max, error:response_unavailable, operation:run.failed_requests
#[test]
fn a_retry_after_past_the_bound_ends_its_domains_requests_failed_unsent() {
    let env = TestEnv::new();

    let (sent, report, failed_requests) = crawled_with(
        &env,
        "maintenance",
        (&json!({}), Duration::ZERO),
        &(
            "response_unavailable",
            true,
            json!({ "status": 503, "retry_after": "2026-01-05T10:00:00Z" }),
        ),
    );

    let failed: Vec<(Value, Value, Value, Value, bool)> = failed_requests["result"]["requests"]
        .as_array()
        .expect("failed requests are listed")
        .iter()
        .map(|request| {
            let error = &request["error"];
            (
                request["request"]["url"].clone(),
                error["code"].clone(),
                error["details"]["status"].clone(),
                error["details"]["retry_after"].clone(),
                error["details"]["fix"]
                    .as_str()
                    .is_some_and(|fix| fix.contains("politeness.retry_after.max")),
            )
        })
        .collect();
    assert_eq!(
        (
            urls(&sent),
            &report["result"]["completeness"],
            &report["result"]["completeness_reasons"][0]["code"],
            failed
        ),
        (
            vec!["https://heavy.test/1", "https://a.test/1"],
            &json!("incomplete"),
            &json!("response_unavailable"),
            vec![
                (
                    json!("https://heavy.test/1"),
                    json!("response_unavailable"),
                    json!(503),
                    json!("2026-01-05T10:00:00Z"),
                    true
                ),
                (
                    json!("https://heavy.test/2"),
                    json!("response_unavailable"),
                    Value::Null,
                    json!("2026-01-05T10:00:00Z"),
                    true
                ),
            ]
        )
    );
}

/// A site that asks for 2 s, past a bound of 1 s, is judged when it asked,
/// never when its handler's failure arrived: a handler that took 1.5 s to
/// report the 503 still ends its request `failed` with no retry spent, and
/// the domain's other request, admitted once that failure came, ends
/// `failed` unsent, though the site's time is by then within the bound,
/// while the other domains go (ADR 0118).
///
/// Covers: story:3-160, setting:politeness.retry_after.max, error:response_unavailable, operation:run.failed_requests
#[test]
fn a_retry_after_past_the_bound_is_judged_when_the_site_asked_however_late_its_failure_comes() {
    let env = TestEnv::new();
    let bound = json!({
        "politeness": {
            "retry_after": { "max": 1_000 },
            "backoff": { "initial": 500, "max": 1_000 },
        },
    });

    let (sent, report, failed_requests) = crawled_with(
        &env,
        "slow-handler",
        (&bound, Duration::from_millis(1_500)),
        &(
            "response_unavailable",
            true,
            json!({ "status": 503, "retry_after": "2026-01-05T09:00:02Z" }),
        ),
    );

    let failed: Vec<(Value, Value)> = failed_requests["result"]["requests"]
        .as_array()
        .expect("failed requests are listed")
        .iter()
        .map(|request| {
            (
                request["request"]["url"].clone(),
                request["error"]["details"]["status"].clone(),
            )
        })
        .collect();
    assert_eq!(
        (urls(&sent), &report["result"]["completeness"], failed),
        (
            vec!["https://heavy.test/1", "https://a.test/1"],
            &json!("incomplete"),
            vec![
                (json!("https://heavy.test/1"), json!(503)),
                (json!("https://heavy.test/2"), Value::Null),
            ]
        )
    );
}
