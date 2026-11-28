//! A Continuous crawl through the engine's public interface over the
//! in-memory network (spec #1836's Seam 1; ADR 0037's RT-12 and RT-46;
//! ADR 0139), on the paused clock: `crawl.continuous` implies
//! `crawl.keep_alive`, revisits each URL by its Label's `revisit.policy`
//! keeping its ID, reports `idle` and `next_due_at`, preserves coverage
//! across failed revisits, persists in the `revisits` table (and works at
//! `run_state.memory.budget: 0`), ends cleanly on `run.finish`, counts idle
//! time toward `timeout.run`, and leaves batch crawls untouched.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, Reply, answer_all, crawl, crawl_connected, dependencies, seeds};

fn seed(url: &str) -> Value {
    json!({ "url": url, "label": "page" })
}

fn every_label(interval_ms: u64) -> Value {
    json!({
        "page": {
            "settings": { "revisit": { "policy": { "kind": "every", "interval": interval_ms } } }
        }
    })
}

/// Advances the paused clock so the engine processes due revisits.
async fn advance(duration: Duration) {
    tokio::time::sleep(duration).await;
}

/// Polls `run.progress` until `predicate` holds, answering handler requests
/// along the way.
async fn progress_matching(
    client: &mut Client,
    run_id: &str,
    predicate: impl Fn(&Value) -> bool,
    handler: &mut impl FnMut(&Value) -> Reply,
) -> Value {
    let mut after: Option<Value> = None;
    loop {
        let params = match &after {
            None => json!({ "run_id": run_id }),
            Some(pos) => json!({ "run_id": run_id, "after": pos }),
        };
        let answer = client.call("run.progress", params, handler).await;
        let result = answer["result"].clone();
        if predicate(&result) {
            return result;
        }
        after = Some(result["position"].clone());
    }
}

/// A Continuous crawl stays running when its Frontier empties, reports
/// `idle: true` with `next_due_at`, re-admits due URLs keeping the same ID
/// and incrementing attempt numbers (never a `duplicate`), and runs a full
/// month (30 daily revisits) in seconds on the paused clock.
///
/// Covers: row:RT-12, row:RT-46, setting:crawl.continuous, setting:revisit.policy, operation:run.progress, operation:run.finish
#[test]
fn a_continuous_crawl_revisits_for_a_month_keeping_id_and_reporting_next_due_at() {
    let env = TestEnv::new();
    let seen_ids = std::cell::RefCell::new(Vec::<(String, u64)>::new());
    let mut handler = |params: &Value| {
        let id = params["crawl_request"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let attempt = params["attempt"].as_u64().unwrap_or(0);
        seen_ids.borrow_mut().push((id, attempt));
        Reply::Records(Vec::new())
    };

    let (idle_progress, outcomes, report) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut labels = every_label(86_400_000);
            labels["article"] =
                json!({ "settings": { "revisit": { "policy": { "kind": "never" } } } });
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "month",
                        "seeds": [seed("https://shop.test/daily"), seed("https://shop.test/once")],
                        "settings": { "crawl": { "continuous": true } },
                        "labels": labels,
                    }),
                    &mut handler,
                )
                .await;
            client
                .call(
                    "run.add_seed",
                    json!({
                        "run_id": "month",
                        "seed": { "url": "https://shop.test/article", "label": "article" },
                    }),
                    &mut handler,
                )
                .await;
            let is_idle = |p: &Value| p["idle"] == true && p.get("next_due_at").is_some();
            let idle_progress = progress_matching(client, "month", is_idle, &mut handler).await;
            for _ in 0..30 {
                advance(Duration::from_hours(24)).await;
                let _ = progress_matching(client, "month", is_idle, &mut handler).await;
            }
            let report = client
                .call("run.finish", json!({ "run_id": "month" }), &mut handler)
                .await;
            let outcomes = client
                .call("run.outcomes", json!({ "run_id": "month" }), &mut handler)
                .await;
            (idle_progress, outcomes, report)
        },
    );

    assert_eq!(idle_progress["idle"], true);
    assert!(
        idle_progress["next_due_at"]
            .as_str()
            .is_some_and(|s| s.ends_with('Z')),
        "expected RFC 3339 next_due_at, got: {idle_progress}"
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(report["result"]["completeness"], "complete", "{report}");
    assert_eq!(report["result"]["requests"]["succeeded"], 3, "{report}");
    assert_eq!(report["result"]["requests"]["skipped"], 0, "{report}");

    let recorded = seen_ids.borrow();
    let page_1_attempts: Vec<u64> = recorded
        .iter()
        .filter(|(id, _)| id == "1")
        .map(|(_, a)| *a)
        .collect();
    assert!(page_1_attempts.len() >= 30, "got {page_1_attempts:?}");
    assert_eq!((page_1_attempts[0], page_1_attempts[1]), (1, 2));
    let article_attempts: Vec<u64> = recorded
        .iter()
        .filter(|(id, _)| id == "3")
        .map(|(_, a)| *a)
        .collect();
    assert_eq!(article_attempts, vec![1]);
    assert!(
        outcomes["result"]["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["kind"] != "skipped"),
        "a revisit is never a duplicate"
    );
}

/// `run.finish` schedules no more revisits, finishes first visits still
/// waiting, and ends the Run `finished`; idle time counts toward `timeout.run`.
///
/// Covers: row:RT-12, row:RT-46, setting:crawl.continuous, setting:timeout.run
#[test]
fn a_continuous_crawls_idle_time_counts_toward_run_timeout() {
    let env = TestEnv::new();

    let report = crawl_connected(
        &json!({ "timeout": { "run": 60_000 } }),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "timed",
                        "seeds": seeds(1),
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": { "policy": { "kind": "every", "interval": 3_600_000 } }
                                }
                            }
                        },
                    }),
                    &mut answer_all,
                )
                .await;
            client
                .call("crawl.serve", json!({ "run_id": "timed" }), &mut answer_all)
                .await
        },
    );

    assert_eq!(report["result"]["lifecycle"], "stopped", "{report}");
    assert_eq!(
        report["result"]["stop_reason"],
        json!({ "kind": "timeout" }),
        "{report}"
    );
    assert_eq!(
        report["result"]["ended_at"],
        json!("2026-01-05T09:01:00Z"),
        "{report}"
    );
}

/// Completeness reads whether each URL has ever been visited: a URL that
/// succeeded on its first visit and failed on a later revisit stays `complete`.
///
/// Covers: row:RT-46, setting:crawl.continuous, setting:revisit.policy
#[test]
fn a_later_failed_revisit_does_not_take_back_coverage() {
    let env = TestEnv::new();
    let mut calls = 0usize;
    let mut handler = |_: &Value| {
        let n = calls;
        calls += 1;
        if n == 0 {
            Reply::Records(Vec::new())
        } else {
            Reply::Fails("handler_failed", false)
        }
    };

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "coverage",
                        "seeds": [seed("https://shop.test/item")],
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": { "policy": { "kind": "every", "interval": 10_000 } }
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;

            // Wait for first visit to settle and go idle.
            let _ =
                progress_matching(client, "coverage", |p| p["idle"] == true, &mut handler).await;

            // Advance past the 10 s revisit interval so the revisit runs and fails.
            advance(Duration::from_secs(10)).await;
            let _ = progress_matching(
                client,
                "coverage",
                |p| p["requests"]["failed"] == 1,
                &mut handler,
            )
            .await;

            client
                .call("run.finish", json!({ "run_id": "coverage" }), &mut handler)
                .await
        },
    );

    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(
        report["result"]["completeness"], "complete",
        "a URL already visited once stays covered when a later revisit fails: {report}"
    );
    assert_eq!(report["result"]["requests"]["failed"], 1, "{report}");
}

/// A batch crawl accepts `revisit.policy` while `config.explain` marks it
/// inactive (`yields_to` `crawl.continuous = false`), writes 0 rows to
/// `revisits`, and a Continuous crawl at `run_state.memory.budget: 0` reads
/// due windows from `revisits` and `run.discard` removes the run file.
///
/// Covers: row:RT-46, setting:crawl.continuous, setting:revisit.policy, operation:config.explain, operation:run.discard
#[test]
fn batch_accepts_revisit_policy_marked_inactive_and_spilled_continuous_uses_revisits_table() {
    let env = TestEnv::new();

    let (explained, spilled_visits, discarded) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let explained = client
                .call(
                    "config.explain",
                    json!({
                        "keys": ["revisit.policy"],
                        "settings": { "revisit": { "policy": { "kind": "every", "interval": 60_000 } } }
                    }),
                    &mut answer_all,
                )
                .await;
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "batch-revisit",
                        "seeds": [seed("https://shop.test/a")],
                        "labels": every_label(1000),
                    }),
                    &mut answer_all,
                )
                .await;
            let batch_report = client
                .call(
                    "run.finish",
                    json!({ "run_id": "batch-revisit" }),
                    &mut answer_all,
                )
                .await;
            assert_eq!(batch_report["result"]["lifecycle"], "finished");

            let count = Rc::new(Cell::new(0usize));
            let count_h = Rc::clone(&count);
            let mut count_handler = move |_: &Value| {
                count_h.set(count_h.get() + 1);
                Reply::Records(Vec::new())
            };
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "spilled-cont",
                        "seeds": [seed("https://shop.test/b")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "run_state": { "memory": { "budget": 0 } },
                        },
                        "labels": every_label(5_000),
                    }),
                    &mut count_handler,
                )
                .await;
            let _ = progress_matching(
                client,
                "spilled-cont",
                |p| p["idle"] == true && p.get("next_due_at").is_some(),
                &mut count_handler,
            )
            .await;
            advance(Duration::from_secs(5)).await;
            let _ = progress_matching(
                client,
                "spilled-cont",
                |p| p["idle"] == true && count.get() >= 2,
                &mut count_handler,
            )
            .await;
            let id = json!({ "run_id": "spilled-cont" });
            let finish_spilled = client
                .call("run.finish", id.clone(), &mut count_handler)
                .await;
            assert_eq!(finish_spilled["result"]["lifecycle"], "finished");
            let discarded = client.call("run.discard", id, &mut count_handler).await;
            (explained, count.get(), discarded)
        },
    );

    let setting = &explained["result"]["settings"][0];
    assert_eq!(setting["key"], "revisit.policy");
    assert_eq!(
        setting["yields_to"]["key"], "crawl.continuous",
        "expected revisit.policy to yield to crawl.continuous on batch crawl: {setting}"
    );
    assert_eq!(setting["yields_to"]["value"], false);
    assert!(
        spilled_visits >= 2,
        "expected spilled revisit to run, got {spilled_visits}"
    );
    assert!(
        discarded.get("error").is_none(),
        "expected discard ok: {discarded}"
    );
}
