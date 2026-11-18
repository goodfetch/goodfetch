//! A crawl paused or moved, then resumed, through the engine's public
//! interface over the in-memory network (spec #213's Seam 3; ADRs 0002,
//! 0003 and 0102): the resume takes the next run epoch and starts the
//! crawl's delivery again on the resuming Client, so the work items its
//! committed state left undone are delivered again, an attempt that state
//! shows in flight counting as `handler_lost`, and those it committed are
//! not, so each Record is committed once.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    POLICY, Reply, answer_all, configure, crawl, default_dependencies, delivered, page_of, records,
    run_file, seeds, work_items,
};
use crate::policy::CrawlPolicy;

/// One handler at a time, so the work items go one after another.
const ONE_AT_A_TIME: CrawlPolicy = CrawlPolicy {
    handler_concurrency: 1,
    ..POLICY
};

/// The Run's settings at the Durability level `level` (ADR 0142): a stop
/// writes everything at each one, so its resume fetches no page that had
/// ended.
fn at(level: &str) -> Value {
    json!({ "run_state": { "durability": level } })
}

/// Both machines' folders lie in memory (`TestEnv::in_memory`): on a busy
/// disk their two run files, the Checkpoint and its copy took it past half
/// the Quick kill (fix-queue 123).
///
/// Covers: story:3-34, operation:run.resume, setting:run_state.durability
#[rstest]
fn a_crawl_resumed_elsewhere_from_its_checkpoint_redoes_only_the_work_since_and_commits_each_record_once(
    #[values("batch", "full", "fast", "periodic", "off")] level: &str,
) {
    let (first, second) = (TestEnv::in_memory(), TestEnv::in_memory());
    let taken = first.folder().join("moved.sqlite");
    let location = taken.display().to_string();
    let checkpoint = location.clone();
    crawl(
        || {
            configure(&first, ONE_AT_A_TIME);
            default_dependencies(&first, RunStateOptions::default())
        },
        async |client| {
            let mut handler = |params: &Value| {
                if page_of(params).ends_with("/1") {
                    answer_all(params)
                } else {
                    Reply::Silent
                }
            };
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "moved", "seeds": seeds(3), "settings": at(level) }),
                    &mut handler,
                )
                .await;
            // Checkpoint until page 1 is committed and page 2 is in flight,
            // each call answering the handler requests that came meanwhile.
            loop {
                client
                    .call(
                        "run.checkpoint",
                        json!({ "run_id": "moved", "location": location }),
                        &mut handler,
                    )
                    .await;
                let file = rusqlite::Connection::open(&taken).unwrap();
                let states: Vec<String> = work_items(&file)
                    .into_iter()
                    .map(|(_, status, _, _)| status)
                    .collect();
                if states == ["succeeded", "leased", "pending"] {
                    break;
                }
            }
        },
    );

    let (resumed, finished, handled) = crawl(
        || {
            configure(&second, ONE_AT_A_TIME);
            default_dependencies(&second, RunStateOptions::default())
        },
        async |client| {
            let resumed = client
                .call(
                    "run.resume",
                    json!({ "run_id": "moved", "checkpoint": checkpoint }),
                    &mut answer_all,
                )
                .await;
            let finished = client
                .call("run.finish", json!({ "run_id": "moved" }), &mut answer_all)
                .await;
            (resumed, finished, delivered(client))
        },
    );

    let file = run_file(&second, "moved");
    let pages: Vec<Value> = records(&file)
        .into_iter()
        .map(|(_, _, _, data)| data)
        .collect();
    assert_eq!(
        (
            resumed["result"]["started_as"].clone(),
            handled,
            finished["result"]["lifecycle"].clone(),
            finished["result"]["completeness"].clone(),
            pages,
            work_items(&file),
        ),
        (
            json!("resumed"),
            vec![
                ("https://shop.test/page/2".to_owned(), 2),
                ("https://shop.test/page/3".to_owned(), 1),
            ],
            json!("finished"),
            json!("complete"),
            vec![
                json!({ "page": "https://shop.test/page/1" }),
                json!({ "page": "https://shop.test/page/2" }),
                json!({ "page": "https://shop.test/page/3" }),
            ],
            vec![
                ("1".to_owned(), "succeeded".to_owned(), 1, None),
                ("2".to_owned(), "succeeded".to_owned(), 2, None),
                ("3".to_owned(), "succeeded".to_owned(), 1, None),
            ],
        )
    );
}

/// The crawl a pause stopped resumes by its Run ID on the same machine and
/// finishes, each Record committed once across the pause.
/// Covers: story:3-35, operation:run.pause, operation:run.resume, setting:run_state.durability
#[rstest]
fn a_paused_crawl_resumes_by_its_run_id_and_finishes_with_each_record_once(
    #[values("batch", "full", "fast", "periodic", "off")] level: &str,
) {
    let env = TestEnv::new();
    let (paused, finished, handled) = crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            let mut first_only = |params: &Value| {
                if page_of(params).ends_with("/1") {
                    answer_all(params)
                } else {
                    Reply::Silent
                }
            };
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "paused", "seeds": seeds(2), "settings": at(level) }),
                    &mut first_only,
                )
                .await;
            let paused = loop {
                let paused = client
                    .call("run.pause", json!({ "run_id": "paused" }), &mut first_only)
                    .await;
                if records(&run_file(&env, "paused")).len() == 1 {
                    break paused;
                }
                client
                    .call("run.resume", json!({ "run_id": "paused" }), &mut first_only)
                    .await;
            };
            client.handled.clear();
            client
                .call("run.resume", json!({ "run_id": "paused" }), &mut answer_all)
                .await;
            let finished = client
                .call("run.finish", json!({ "run_id": "paused" }), &mut answer_all)
                .await;
            (paused, finished, delivered(client))
        },
    );

    let pages: Vec<Value> = records(&run_file(&env, "paused"))
        .into_iter()
        .map(|(_, _, _, data)| data)
        .collect();
    assert_eq!(
        (
            paused["result"]["stop_reason"].clone(),
            handled,
            finished["result"]["lifecycle"].clone(),
            pages,
        ),
        (
            json!({ "kind": "pause" }),
            vec![("https://shop.test/page/2".to_owned(), 1)],
            json!("finished"),
            vec![
                json!({ "page": "https://shop.test/page/1" }),
                json!({ "page": "https://shop.test/page/2" }),
            ],
        )
    );
}

/// A crawl its `timeout.run` stopped writes a Checkpoint at that stop, and
/// its resume, on a fresh clock with no timeout, opens its admission from
/// the per-domain journal and delivers only the work item left pending.
/// Covers: story:3-30, story:3-35, operation:run.resume
#[test]
fn a_crawl_its_timeout_stopped_checkpoints_then_resumes_only_its_pending_work() {
    let env = TestEnv::new();
    let seeds: Value = ["https://a.test/1", "https://b.test/1", "https://c.test/1"]
        .iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect();
    let (stopped, handled, finished) = crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "timed",
                        "seeds": seeds,
                        "settings": {
                            "timeout": { "run": 15_000 },
                            "admission": { "max_requests_per_minute": 6 },
                        },
                    }),
                    &mut answer_all,
                )
                .await;
            let stopped = client
                .call("run.finish", json!({ "run_id": "timed" }), &mut answer_all)
                .await;
            client.handled.clear();
            client
                .call("run.resume", json!({ "run_id": "timed" }), &mut answer_all)
                .await;
            let finished = client
                .call("run.finish", json!({ "run_id": "timed" }), &mut answer_all)
                .await;
            (stopped, delivered(client), finished)
        },
    );

    assert_eq!(
        (
            stopped["result"]["stop_reason"].clone(),
            stopped["result"]["checkpoint"]["run_epoch"].clone(),
            handled,
            finished["result"]["lifecycle"].clone(),
            work_items(&run_file(&env, "timed")),
        ),
        (
            json!({ "kind": "timeout" }),
            json!(1),
            vec![("https://c.test/1".to_owned(), 1)],
            json!("finished"),
            vec![
                ("1".to_owned(), "succeeded".to_owned(), 1, None),
                ("2".to_owned(), "succeeded".to_owned(), 1, None),
                ("3".to_owned(), "succeeded".to_owned(), 1, None),
            ],
        )
    );
}
