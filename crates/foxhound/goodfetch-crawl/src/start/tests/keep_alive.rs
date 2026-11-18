//! A keep-alive crawl through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADR 0037's RT-12, ADR 0102), on
//! the paused clock: with `crawl.keep_alive` the Run stays `running` once
//! its Frontier is empty and nothing is in flight, waiting for new work,
//! until `run.finish` lets it end, `finished`; its idle time counts toward
//! its `timeout.run`, and an abort stops it as any Run.

use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, Reply, answer_all, crawl, dependencies, seeds};

/// Starts the keep-alive crawl `run_id` of one seed with `settings` beside
/// `crawl.keep_alive`, and waits until its seed's outcome is committed.
async fn started_and_drained(client: &mut Client, run_id: &str, mut settings: Value) {
    settings["crawl"] = json!({ "keep_alive": true });
    client
        .call(
            "crawl.start",
            json!({ "run_id": run_id, "seeds": seeds(1), "settings": settings, "labels": { "page": {} } }),
            &mut |_| Reply::Silent,
        )
        .await;
    let page = client
        .call("run.outcomes", json!({ "run_id": run_id }), &mut answer_all)
        .await;
    assert_eq!(page["result"]["outcomes"][0]["kind"], "succeeded", "{page}");
}

/// The paused clock moves on only once the engine has nothing left to do,
/// so the crawl has settled into its wait for new work by then.
async fn idle_for(duration: Duration) {
    tokio::time::sleep(duration).await;
}

/// Covers: story:3-128, setting:crawl.keep_alive
#[test]
fn a_keep_alive_crawl_stays_running_once_drained_until_run_finish_lets_it_end() {
    let env = TestEnv::new();

    let (while_idle, report) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            started_and_drained(client, "served", json!({})).await;
            idle_for(Duration::from_mins(1)).await;
            let while_idle = client
                .call(
                    "crawl.start",
                    json!({ "run_id": "served", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let report = client
                .call("run.finish", json!({ "run_id": "served" }), &mut answer_all)
                .await;
            (while_idle, report)
        },
    );

    assert_eq!(
        (
            &while_idle["result"]["lifecycle"],
            (
                &report["result"]["lifecycle"],
                &report["result"]["completeness"],
                &report["result"]["ended_at"],
            ),
        ),
        (
            &json!("running"),
            (
                &json!("finished"),
                &json!("complete"),
                &json!("2026-01-05T09:01:00Z")
            ),
        )
    );
}

/// `crawl.serve` waits for the Run's end without letting it finish, so
/// only the `timeout.run` of a minute ends the idle crawl.
///
/// Covers: story:3-128, setting:crawl.keep_alive, setting:timeout.run
#[test]
fn a_keep_alive_crawls_idle_time_counts_toward_its_run_timeout() {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            started_and_drained(client, "idle", json!({ "timeout": { "run": 60_000 } })).await;
            client
                .call("crawl.serve", json!({ "run_id": "idle" }), &mut answer_all)
                .await
        },
    );

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
            &report["result"]["ended_at"],
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "timeout" }),
            &json!("2026-01-05T09:01:00Z"),
        )
    );
}

/// Covers: story:3-128, operation:run.abort
#[test]
fn an_idle_keep_alive_crawl_aborted_stops_incomplete() {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            started_and_drained(client, "dropped", json!({})).await;
            idle_for(Duration::from_mins(1)).await;
            client
                .call("run.abort", json!({ "run_id": "dropped" }), &mut answer_all)
                .await
        },
    );

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "abort", "by": "client" }),
        )
    );
}
