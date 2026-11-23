//! A crawl's admission through the engine's public interface over the
//! in-memory network (spec #213's Seam 3), on the paused clock: the order
//! the per-domain queues give its work items under each
//! `crawl.domain_order`, and the share of the requests in flight a slow
//! domain holds under it (#2197), the rate cap's spacing and
//! the queues' journal in the Run's file; the backoff and the `Retry-After`
//! a handler met, which hold its domain in the Politeness the Run shares,
//! a `Retry-After` past `politeness.retry_after.max`, which ends the
//! domain's requests `failed` unsent, and one that does not parse, which
//! the Run trace notes (ADR 0118).

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::time::Instant;

use goodfetch_protocol::home::RUN_TRACE_REQUESTS;

use super::{
    RETRY_AFTER, Reply, as_sent, crawl, default_dependencies, dependencies, page_of, run_file,
    run_folder,
};

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
                    json!({ "run_id": "fair", "seeds": seeds, "settings": settings, "labels": { "page": {} } }),
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

/// One request in flight at a time leaves every domain key with none in
/// flight at each choice, so both orders give the turns.
///
/// `settings` with `crawl.domain_order` at `order`, when one is given.
fn ordered(mut settings: Value, order: Option<&str>) -> Value {
    if let Some(order) = order {
        settings["crawl"]["domain_order"] = json!(order);
    }
    settings
}

/// One handler request: its URL, and when it came and when it was
/// answered, each as the paused clock's time since the first request.
#[derive(Debug, PartialEq, Eq)]
struct Answered {
    url: String,
    came: Duration,
    ended: Duration,
}

/// Starts `seeds` under `settings` and finishes the crawl, answering each
/// handler request once `took` of its URL passed on the paused clock while
/// the others come and are answered meanwhile, as handlers that take that
/// long. Returns the requests, as they came. A handler has the settings'
/// default time, 60 s.
fn answered_after(
    env: &TestEnv,
    seeds: &Value,
    settings: &Value,
    took: impl Fn(&str) -> Duration,
) -> Vec<Answered> {
    crawl(
        || default_dependencies(env, RunStateOptions::default()),
        async |client| {
            let params = json!({ "run_id": "timed", "seeds": seeds, "labels": { "page": {} }, "settings": settings });
            client
                .send(&json!({ "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": as_sent("crawl.start", params) }))
                .await;
            // Each request's answer, by when it is due and then by when
            // the request came, with the request's place in `came`.
            let mut due: BTreeMap<(Instant, usize), Value> = BTreeMap::new();
            let mut came: Vec<(String, Instant, Option<Instant>)> = Vec::new();
            loop {
                let next = match due.first_key_value() {
                    Some(((at, _), _)) => tokio::time::timeout_at(*at, client.next()).await,
                    None => Ok(client.next().await),
                };
                let Ok(message) = next else {
                    let Some(((_, index), id)) = due.pop_first() else {
                        continue;
                    };
                    let (url, _, answered) = &mut came[index];
                    *answered = Some(Instant::now());
                    let records = json!([{ "dataset": "pages", "data": { "page": url } }]);
                    client
                        .send(&json!({ "jsonrpc": "2.0", "id": id, "result": { "records": records } }))
                        .await;
                    continue;
                };
                let message = message.expect("the engine closed before the crawl finished");
                if message["method"] == "crawl_request.handle" {
                    let url = page_of(&message["params"]);
                    let now = Instant::now();
                    due.insert((now + took(&url), came.len()), message["id"].clone());
                    came.push((url, now, None));
                } else if message["id"] == "start" {
                    client
                        .send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "timed" } }))
                        .await;
                } else if message["id"] == "finish" {
                    assert_eq!(message["result"]["lifecycle"], "finished", "{message}");
                    break;
                }
            }
            let first = came.first().map(|(_, at, _)| *at).expect("a request came");
            came.into_iter()
                .map(|(url, at, answered)| Answered {
                    url,
                    came: at - first,
                    ended: answered.expect("every request was answered") - first,
                })
                .collect()
        },
    )
}

/// The most requests to `host` in flight at once when one of them came
/// while another host's requests were still to come.
fn most_in_flight_while_others_wait(requests: &[Answered], host: &str) -> usize {
    let of_host = |request: &&Answered| request.url.contains(&format!("://{host}/"));
    let last_other = requests
        .iter()
        .filter(|request| !of_host(request))
        .map(|request| request.came)
        .max()
        .unwrap_or_default();
    requests
        .iter()
        .filter(of_host)
        .filter(|request| request.came < last_other)
        .map(|request| {
            requests
                .iter()
                .filter(of_host)
                .filter(|other| other.came <= request.came && request.came < other.ended)
                .count()
        })
        .max()
        .unwrap_or_default()
}

/// Covers: story:3-47, setting:crawl.domain_order
#[rstest]
#[case::by_default(None)]
#[case::round_robin(Some("round_robin"))]
fn a_heavy_domain_listed_first_lets_the_small_ones_take_their_turns(#[case] order: Option<&str>) {
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
        &ordered(
            json!({ "crawl": { "handler": { "concurrency": 1 } } }),
            order,
        ),
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
                    json!({ "run_id": "slow", "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 1 } } }, "labels": { "page": {} } }),
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
/// user shares, or in the one the Run keeps to itself in memory (#2171):
/// the domain's next request waits its first backoff, from 2 s to 6 s,
/// while the others go (ADRs 0021, 0053 and 0118).
///
/// Covers: story:3-80, setting:politeness.sharing
#[rstest::rstest]
fn a_429_a_handler_met_backs_its_domain_off_while_the_others_go(
    #[values("shared", "run")] sharing: &str,
) {
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
                    json!({ "run_id": "refused", "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 1 } }, "politeness": { "sharing": sharing } }, "labels": { "page": {} } }),
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

/// A crawl's Politeness switched off: no delay, no per-domain limit, no
/// rate, and robots.txt not obeyed, so no Crawl-delay paces it (ADR 0138),
/// beside `more`.
fn politeness_off(more: &Value) -> Value {
    let mut settings = json!({
        "politeness": { "domain": { "delay": 0, "concurrency": null } },
        "site_preferences": { "respect_robots_txt": false },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        for (key, value) in more {
            match (settings.get_mut(key), value) {
                (Some(Value::Object(into)), Value::Object(value)) => {
                    into.extend(value.clone());
                }
                _ => {
                    settings.insert(key.clone(), value.clone());
                }
            }
        }
    }
    settings
}

/// A crawl whose Politeness is switched off consults no Politeness store:
/// the per-user Politeness file is never opened, so never made, and every
/// request goes at once (ADR 0138).
#[test]
fn a_crawl_with_politeness_off_never_opens_the_politeness_file() {
    let env = TestEnv::new();
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://heavy.test/2"),
        seed("https://a.test/1"),
    ]);

    let handled = handled(
        &env,
        &seeds,
        &politeness_off(&json!({ "crawl": { "handler": { "concurrency": 1 } } })),
    );

    assert_eq!(
        (
            urls(&handled).len(),
            env.home().join("politeness.sqlite").exists()
        ),
        (3, false)
    );
}

/// The handler requests sent to `heavy.test` after its first, which met
/// the site's 429, by a crawl one request at a time under `settings`, each
/// with the paused clock's time since the first request.
fn after_a_429(env: &TestEnv, run_id: &str, settings: &Value) -> Vec<(String, Duration)> {
    let seeds = json!([
        seed("https://heavy.test/1"),
        seed("https://heavy.test/2"),
        seed("https://a.test/1"),
    ]);
    let mut settings = settings.clone();
    settings["crawl"] = json!({ "handler": { "concurrency": 1 } });
    let sent = crawl(
        || dependencies(env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": run_id, "seeds": seeds, "settings": settings, "labels": { "page": {} } }),
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
                .call("run.finish", json!({ "run_id": run_id }), &mut handler)
                .await;
            sent(client)
        },
    );
    sent.into_iter()
        .filter(|(url, _)| url == "https://heavy.test/2")
        .collect()
}

/// With Politeness switched off, the backoff stays its own feature: a 429 a
/// handler met still backs its domain off, its first wait drawn between
/// `politeness.backoff.initial` and three times it, and with
/// `politeness.backoff.enabled` off the domain's next request goes at once
/// (ADRs 0118 and 0138).
#[rstest::rstest]
/// Covers: story:3-80, setting:politeness.backoff.initial
#[case::backoff_on(
    json!({ "politeness": { "backoff": { "initial": 10_000, "max": 60_000 } } }),
    Duration::from_secs(10)..=Duration::from_secs(30),
)]
/// Covers: setting:politeness.backoff.enabled
#[case::backoff_off(
    json!({ "politeness": { "backoff": { "enabled": false } } }),
    Duration::ZERO..=Duration::ZERO,
)]
fn with_politeness_off_a_429_backs_its_domain_off_as_the_backoff_says(
    #[case] backoff: Value,
    #[case] waited: std::ops::RangeInclusive<Duration>,
) {
    let env = TestEnv::new();

    let sent = after_a_429(&env, "off", &politeness_off(&backoff));

    assert!(
        sent.len() == 1 && waited.contains(&sent[0].1),
        "{sent:?} not within {waited:?}"
    );
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
                    json!({ "run_id": run_id, "seeds": seeds, "settings": settings, "labels": { "page": {} } }),
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
/// domain until the time the site asked, 30 s, past any first backoff, in
/// the Politeness the user's runs share or in the one the Run keeps to
/// itself in memory (#2171): nothing more is sent to it before then, while
/// the other domains go (ADR 0118).
///
/// Covers: story:3-159, setting:politeness.sharing
#[rstest::rstest]
#[case::too_many_requests(("blocked", false, json!({ "cause": "rate_limit", "retry_after": "2026-01-05T09:00:30Z" })))]
#[case::unavailable(("response_unavailable", true, json!({ "status": 503, "retry_after": "2026-01-05T09:00:30Z" })))]
fn a_retry_after_a_handler_met_holds_its_domain_until_the_time_the_site_asked(
    #[case] failure: (&'static str, bool, Value),
    #[values("shared", "run")] sharing: &str,
) {
    let env = TestEnv::new();
    let settings = json!({ "politeness": { "sharing": sharing } });

    let (sent, _, _) = crawled_with(&env, "asked", (&settings, Duration::ZERO), &failure);

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

/// With Politeness switched off, a site's `Retry-After` a handler met with
/// its 429 still holds the domain until the time the site asked, 30 s,
/// while the other domains go (ADRs 0118 and 0138).
///
/// Covers: story:3-159
#[test]
fn with_politeness_off_a_retry_after_still_holds_its_domain_until_the_time_the_site_asked() {
    let env = TestEnv::new();

    let (sent, _, _) = crawled_with(
        &env,
        "asked-off",
        (&politeness_off(&json!({})), Duration::ZERO),
        &(
            "blocked",
            false,
            json!({ "cause": "rate_limit", "retry_after": "2026-01-05T09:00:30Z" }),
        ),
    );

    let to_heavy: Vec<Duration> = sent
        .iter()
        .filter(|(url, _)| url.starts_with("https://heavy.test/"))
        .map(|(_, at)| *at)
        .collect();
    assert!(
        to_heavy.len() == 2
            && to_heavy[0] == Duration::ZERO
            && to_heavy[1] >= Duration::from_secs(30)
            && sent.contains(&("https://a.test/1".to_owned(), Duration::ZERO)),
        "{sent:?}"
    );
}

/// The data of each note of a `Retry-After` that does not parse in the
/// request log of `run_id`'s Run trace, in order; none when the Run wrote
/// no request log.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's request log once its engine stopped, on no async worker"
)]
fn unparsable_retry_afters(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let log = match std::fs::read_to_string(run_folder(env, run_id).join(RUN_TRACE_REQUESTS)) {
        Ok(log) => log,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => panic!("the request log cannot be read: {error}"),
    };
    log.lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["event"] == "crawl.retry_after_unparsable")
        .map(|line| line["data"].clone())
        .collect()
}

/// Crawls `heavy.test`'s page at `path`, its `/2` and `a.test/1`, one
/// handler request at a time; answers the handler requests sent, as
/// [`sent`] does, the ID of the request of `path`, and the notes of a
/// `Retry-After` that does not parse.
fn crawled_at(
    env: &TestEnv,
    run_id: &str,
    path: &str,
) -> (Vec<(String, Duration)>, Value, Vec<Value>) {
    let url = format!("https://heavy.test{path}");
    let seeds = json!([
        seed(&url),
        seed("https://heavy.test/2"),
        seed("https://a.test/1")
    ]);
    let mut id = Value::Null;
    let sent = crawl(
        || dependencies(env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": run_id, "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 1 } } }, "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if page_of(params) == url {
                    id = params["crawl_request"]["id"].clone();
                }
                Reply::Records(vec![json!({ "page": page_of(params) })])
            };
            client
                .call("run.finish", json!({ "run_id": run_id }), &mut handler)
                .await;
            sent(client)
        },
    );
    (sent, id, unparsable_retry_afters(env, run_id))
}

/// A `Retry-After` that does not parse, on the 503 of a request's own
/// document or on a redirect's 302, is noted once in the Run trace's
/// request log, naming the request, the URL that answered, its status and
/// the value as the site sent it; it adds no wait, so after the 503 the
/// domain's next request waits only its first backoff (ADR 0118).
///
/// Covers: story:3-159
#[rstest]
fn a_retry_after_that_does_not_parse_is_noted_and_adds_no_wait(#[values(503, 302)] status: u16) {
    let env = TestEnv::new();
    let path = format!("{RETRY_AFTER}{status}/soon");

    let (sent, id, notes) = crawled_at(&env, "unparsable", &path);

    assert!(id.is_string(), "{sent:?}");
    assert_eq!(
        notes,
        [json!({
            "crawl_request_id": id,
            "url": format!("https://heavy.test{path}"),
            "status": status,
            "retry_after": "soon",
        })]
    );
    if status == 503 {
        let at = |url: &str| {
            sent.iter()
                .find(|(sent, _)| sent == url)
                .map(|(_, at)| *at)
                .expect("the request was sent")
        };
        let waited = at("https://heavy.test/2")
            .checked_sub(at(&format!("https://heavy.test{path}")))
            .expect("the next page was sent after the 503");
        assert!(a_first_backoff(waited), "{sent:?}");
    }
}

/// A `Retry-After` of `0`, or of a date already past, parses: it adds no
/// wait and the Run trace notes nothing; nor does it note one the engine
/// does not read, on a document's 200 (ADR 0118).
///
/// Covers: story:3-159
#[rstest]
#[case::zero_on_a_503(503, "zero")]
#[case::a_past_date_on_a_503(503, "past")]
#[case::zero_on_a_302(302, "zero")]
#[case::a_past_date_on_a_302(302, "past")]
#[case::one_that_does_not_parse_on_a_200(200, "soon")]
fn a_retry_after_that_asks_no_wait_is_not_noted(#[case] status: u16, #[case] form: &str) {
    let env = TestEnv::new();

    let (sent, _, notes) = crawled_at(&env, "no-wait", &format!("{RETRY_AFTER}{status}/{form}"));

    assert!(notes.is_empty() && sent.len() == 3, "{notes:?} {sent:?}");
}

/// A sitemap's fetch follows its redirect as a page's does: a `Retry-After`
/// on its 302 that does not parse is noted once, naming the sitemap's
/// crawl request (ADR 0118).
///
/// Covers: story:3-159
#[test]
fn a_sitemap_s_redirect_notes_a_retry_after_that_does_not_parse() {
    let env = TestEnv::new();
    let url = format!("https://heavy.test{RETRY_AFTER}302/soon");
    let seeds = json!([{ "sitemap": url, "rules": [{ "pattern": "**", "label": "page" }] }]);

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "sitemap", "seeds": seeds, "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "sitemap" }),
                    &mut |params| Reply::Records(vec![json!({ "page": page_of(params) })]),
                )
                .await;
        },
    );
    let notes = unparsable_retry_afters(&env, "sitemap");

    assert!(
        matches!(
            notes.as_slice(),
            [note] if note["crawl_request_id"].is_string()
                && note["url"] == url
                && note["status"] == 302
                && note["retry_after"] == "soon"
        ),
        "{notes:?}"
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

/// A slow domain among fast ones: `slow.test`'s handler takes 10 s and the
/// others' 1 s, with 4 requests in flight at most.
fn a_slow_domain_among_fast_ones() -> (Value, impl Fn(&str) -> Duration) {
    let seeds: Vec<Value> = ["slow.test", "a.test", "b.test", "c.test"]
        .into_iter()
        .flat_map(|host| (1..=4).map(move |page| seed(&format!("https://{host}/{page}"))))
        .collect();
    let took = |url: &str| {
        Duration::from_secs(if url.starts_with("https://slow.test/") {
            10
        } else {
            1
        })
    };
    (Value::Array(seeds), took)
}

/// A slow domain holds no more than its share of the requests in flight
/// while the fast ones have requests waiting, one of four, since a free
/// one goes to the domain with the fewest in flight; at `round_robin` it
/// takes one each turn and comes to hold all four, today's order (#2197).
///
/// Covers: story:3-47, setting:crawl.domain_order
#[rstest]
#[case::by_default(None, 1)]
#[case::fewest_in_flight(Some("fewest_in_flight"), 1)]
#[case::round_robin(Some("round_robin"), 4)]
fn a_slow_domain_holds_its_share_of_the_requests_in_flight(
    #[case] order: Option<&str>,
    #[case] most: usize,
) {
    let env = TestEnv::new();
    let (seeds, took) = a_slow_domain_among_fast_ones();

    let requests = answered_after(
        &env,
        &seeds,
        &ordered(
            json!({ "crawl": { "handler": { "concurrency": 4 } } }),
            order,
        ),
        took,
    );

    assert_eq!(
        (
            requests.len(),
            most_in_flight_while_others_wait(&requests, "slow.test")
        ),
        (16, most)
    );
}

/// With one request in flight per domain key, a key with one in flight
/// waits for it, so every key a free request goes to has none: both
/// orders send the same requests at the same moments (#2197).
///
/// Covers: story:3-47, setting:crawl.domain_order, setting:politeness.domain.concurrency
#[test]
fn one_request_per_domain_key_in_flight_gives_the_same_order_either_way() {
    let sent = |order: &str| {
        let env = TestEnv::new();
        let (seeds, took) = a_slow_domain_among_fast_ones();
        let settings = json!({
            "crawl": { "handler": { "concurrency": 4 }, "domain_order": order },
            "politeness": { "domain": { "concurrency": 1 } },
        });
        answered_after(&env, &seeds, &settings, took)
    };

    let fewest = sent("fewest_in_flight");

    assert_eq!(
        (
            most_in_flight_while_others_wait(&fewest, "slow.test"),
            fewest
        ),
        (1, sent("round_robin"))
    );
}
