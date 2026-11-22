//! A kill between an attempt's staging and its commit, on real SQLite
//! through the crash VFS (ADR 0060; TESTING.md, crashes in one process;
//! #339): the attempt staged an enqueue, a seed and a Key-value store write,
//! then the process died or the power went before its handler answered.
//! Nothing it staged is in the run file the next process opens, which
//! passes `PRAGMA integrity_check`; at the default level its lease rode the
//! open batch with them (ADR 0142), so the page is pending there. Once the
//! Run is resumed the page's next attempt stages them again and its commit
//! is the one that counts: each request once, the Frontier's visited set
//! and counts naming exactly the work items, and the key that attempt
//! wrote.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Reply, configure, crawl, default_dependencies, page_of, run_folder, seeds};
use crate::policy::CrawlPolicy;

/// One handler at a time, so the attempts go one after another.
const ONE_AT_A_TIME: CrawlPolicy = CrawlPolicy {
    handler_concurrency: 1,
    ..super::POLICY
};

/// The calls the seed's attempt `params` makes from inside it: an enqueue
/// of `/b`, a seed `/s` through its `ctx.run`, and the key `progress` set
/// to its attempt's number through it.
fn staged(params: &Value) -> Vec<(&'static str, Value)> {
    let run_id = &params["run_id"];
    let attempt = json!({
        "crawl_request_id": params["crawl_request"]["id"],
        "lease_epoch": params["lease_epoch"],
    });
    vec![
        (
            "crawl_request.enqueue",
            json!({
                "run_id": run_id,
                "crawl_request_id": params["crawl_request"]["id"],
                "lease_epoch": params["lease_epoch"],
                "url": "https://shop.test/b",
                "label": "page",
            }),
        ),
        (
            "run.add_seed",
            json!({ "run_id": run_id, "seed": { "url": "https://shop.test/s", "label": "page" }, "attempt": attempt }),
        ),
        (
            "key_value_store.set",
            json!({ "run_id": run_id, "key": "progress", "value": params["attempt"], "attempt": attempt }),
        ),
    ]
}

/// What the run file's Frontier and Key-value store hold: each work item's
/// URL and status, whether the visited set names exactly the work items,
/// how many requests the bounds counted admitted, and each key's value.
type Held = (Vec<(String, String)>, bool, i64, Vec<(String, String)>);

fn held(connection: &Connection) -> Held {
    let work_items = connection
        .prepare("SELECT url, status FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let visited_are_the_work_items: bool = connection
        .query_row(
            "SELECT (SELECT count(DISTINCT crawl_request) FROM visited) = (SELECT count(*) FROM work_items)
                 AND NOT EXISTS (SELECT 1 FROM visited WHERE crawl_request NOT IN (SELECT id FROM work_items))",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let admitted = connection
        .query_row(
            "SELECT coalesce((SELECT count FROM bounds_admitted), 0)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let keys = connection
        .prepare("SELECT key, value FROM key_values ORDER BY key")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (work_items, visited_are_the_work_items, admitted, keys)
}

/// Covers: story:6-236, operation:run.resume
#[rstest]
#[case::process(CrashKind::Process)]
#[case::power_cut(CrashKind::PowerCut)]
fn a_kill_between_an_attempts_staging_and_its_commit_leaves_the_frontier_consistent_after_a_resume(
    #[case] kind: CrashKind,
) {
    let env = TestEnv::new();
    let folder = env
        .folder()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .replace(' ', "-");
    let vfs = CrashVfs::register(&format!("staging-{folder}"), env.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        ..RunStateOptions::default()
    };
    let path = run_folder(&env, "killed")
        .join("run.sqlite")
        .to_string_lossy()
        .into_owned();
    crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, options.clone())
        },
        async |client| {
            let mut stage_then_hang =
                |params: &Value| Reply::Calls(staged(params), Box::new(Reply::Silent));
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "killed", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut stage_then_hang,
                )
                .await;
            while client.replies.len() < 3 {
                client
                    .call(
                        "run.seeds",
                        json!({ "run_id": "killed" }),
                        &mut stage_then_hang,
                    )
                    .await;
            }
            assert!(
                client
                    .replies
                    .iter()
                    .all(|reply| reply.get("result").is_some()),
                "{:?}",
                client.replies
            );
            vfs.crash(kind);
        },
    );
    let crashed = held(&after_crash(&vfs, &path).unwrap());

    let finished = crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, options.clone())
        },
        async |client| {
            let mut stage_then_commit = |params: &Value| {
                let commit = Reply::Records(vec![json!({ "page": page_of(params) })]);
                if page_of(params) == "https://shop.test/page/1" {
                    Reply::Calls(staged(params), Box::new(commit))
                } else {
                    commit
                }
            };
            client
                .call(
                    "run.resume",
                    json!({ "run_id": "killed" }),
                    &mut stage_then_commit,
                )
                .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "killed" }),
                    &mut stage_then_commit,
                )
                .await
        },
    );
    let resumed = held(
        &Connection::open_with_flags_and_vfs(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            vfs.name(),
        )
        .unwrap(),
    );

    let item = |url: &str, status: &str| (url.to_owned(), status.to_owned());
    assert_eq!(
        (crashed, finished["result"]["lifecycle"].clone(), resumed),
        (
            (
                vec![item("https://shop.test/page/1", "pending")],
                true,
                1,
                Vec::new()
            ),
            json!("finished"),
            (
                vec![
                    item("https://shop.test/page/1", "succeeded"),
                    item("https://shop.test/b", "succeeded"),
                    item("https://shop.test/s", "succeeded"),
                ],
                true,
                3,
                vec![("progress".to_owned(), "1".to_owned())],
            ),
        )
    );
}
