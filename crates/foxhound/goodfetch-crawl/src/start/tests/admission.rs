//! A crawl's admission through the engine's public interface over the
//! in-memory network (spec #213's Seam 3), on the paused clock: the order
//! the per-domain queues give its work items, the rate cap's spacing and
//! the queues' journal in the Run's file.

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
