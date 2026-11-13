//! Work a crawl is given while it finishes, through the engine's public
//! interface over the in-memory network, on the paused clock with blocking
//! work inline (ADRs 0024 and 0102): a re-admission or a seed added while
//! the crawl's delivery ends its Run is either taken by that delivery,
//! which then goes on, or refused by it, the call then taking the path
//! of an ended Run. Whichever the call's arrival, its work is never
//! answered and left pending in a finished Run. The cases send the call
//! `turns` of the runtime after `run.finish`, so both paths are taken: on
//! the paused current-thread runtime the crawl's last look and its end
//! fall within one turn, so the order of the two writer jobs that race is
//! checked on the crawl's tables, in `outcomes::tests::readmission`.

use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::json;

use super::{Client, Reply, answer_all, crawl, dependencies, run_file, seeds, work_items};

/// Starts the keep-alive crawl `run_id` of one seed whose handler fails
/// it unretried, waits for its outcome, and lets the crawl settle into
/// its wait for new work.
async fn failed_and_idle(client: &mut Client, run_id: &str) {
    client
        .call(
            "crawl.start",
            json!({
                "run_id": run_id,
                "seeds": seeds(1),
                "settings": { "crawl": { "keep_alive": true } },
            }),
            &mut |_| Reply::Silent,
        )
        .await;
    let page = client
        .call("run.outcomes", json!({ "run_id": run_id }), &mut |_| {
            Reply::Fails("handler_failed", false)
        })
        .await;
    assert_eq!(page["result"]["outcomes"][0]["kind"], "failed", "{page}");
    tokio::time::sleep(Duration::from_mins(1)).await;
}

/// Sends `run.finish` of `run_id`, unanswered, then lets the runtime take
/// `turns` turns.
async fn finishing(client: &mut Client, run_id: &str, turns: usize) {
    client
        .send(&json!({
            "jsonrpc": "2.0", "id": "finishing", "method": "run.finish", "params": { "run_id": run_id },
        }))
        .await;
    for _ in 0..turns {
        turn().await;
    }
}

/// One turn of the runtime: the test's task goes to the back of the tasks
/// ready to run, so each other ready task is polled once before it is
/// again. Tokio's `yield_now` defers the task until the runtime next
/// parks, which lets every task run until none is ready.
async fn turn() {
    let mut woken = false;
    std::future::poll_fn(|context| {
        if woken {
            return std::task::Poll::Ready(());
        }
        woken = true;
        context.waker().wake_by_ref();
        std::task::Poll::Pending
    })
    .await;
}

/// A failed request re-admitted while its crawl finishes is crawled again:
/// by the finishing delivery, or by the Run the re-admission reopens once
/// it ended. It is never answered re-admitted and then left pending.
///
/// Covers: story:6-306, operation:run.retry_failed
#[rstest]
fn a_re_admission_while_the_crawl_finishes_is_crawled(
    #[values(0, 1, 2, 3, 4, 8, 16, 64)] turns: usize,
) {
    let env = TestEnv::new();

    let (retried, report) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            failed_and_idle(client, "retried").await;
            finishing(client, "retried", turns).await;
            let retried = client
                .call(
                    "run.retry_failed",
                    json!({ "run_id": "retried" }),
                    &mut answer_all,
                )
                .await;
            let report = client
                .call(
                    "run.finish",
                    json!({ "run_id": "retried" }),
                    &mut answer_all,
                )
                .await;
            (retried, report)
        },
    );

    assert_eq!(
        (
            &retried["result"],
            &report["result"]["lifecycle"],
            &report["result"]["requests"]["succeeded"],
            &report["result"]["requests"]["pending"],
            work_items(&run_file(&env, "retried")),
        ),
        (
            &json!({ "readmitted": 1 }),
            &json!("finished"),
            &json!(1),
            &json!(0),
            vec![("1".to_owned(), "succeeded".to_owned(), 2, None)],
        )
    );
}

/// A seed added while its crawl finishes is crawled by the finishing
/// delivery, or refused with `run_not_found` as a seed added to an ended
/// crawl is: it is never answered added and then left pending.
///
/// Covers: story:3-128, operation:run.add_seed
#[rstest]
fn a_seed_added_while_the_crawl_finishes_is_crawled_or_refused(
    #[values(0, 1, 2, 3, 4, 8, 16, 64)] turns: usize,
) {
    let env = TestEnv::new();

    let added = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            failed_and_idle(client, "seeded").await;
            finishing(client, "seeded", turns).await;
            let added = client
                .call(
                    "run.add_seed",
                    json!({
                        "run_id": "seeded",
                        "seed": { "url": "https://shop.test/late", "label": "page" },
                    }),
                    &mut answer_all,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "seeded" }), &mut answer_all)
                .await;
            added
        },
    );

    let late: Vec<String> = work_items(&run_file(&env, "seeded"))
        .into_iter()
        .filter(|(id, ..)| id != "1")
        .map(|(_, status, ..)| status)
        .collect();
    let answered = match added.get("result") {
        Some(result) => result["kind"].clone(),
        None => added["error"]["data"]["code"].clone(),
    };
    assert!(
        (answered == json!("added") && late == ["succeeded"])
            || (answered == json!("run_not_found") && late.is_empty()),
        "{added} {late:?}"
    );
}
