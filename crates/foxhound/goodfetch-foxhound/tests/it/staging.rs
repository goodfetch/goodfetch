//! What an attempt staged, on Foxhound's executable over stdio (spec
//! #581's seam: the protocol; ADRs 0093 and 0103): the enqueues, the seeds
//! added through the handler's `ctx.run` and the Key-value store writes
//! made through it are staged with the attempt and commit with its Records,
//! or are discarded with them, so a failed attempt leaves no stray link,
//! seed or key, and the retried attempt's commit is the one that counts.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::{Inside, added, crawl_declaring, crawl_under};

/// The crawl's one seed.
const SEED: &str = "https://shop.test/a";

/// The attempt `handle` gave, as a handler's `ctx.run` names it.
fn attempt_of(handle: &Value) -> Value {
    json!({
        "crawl_request_id": handle["crawl_request"]["id"],
        "lease_epoch": handle["lease_epoch"],
    })
}

/// The result of `reply`, else its Error code.
fn answer_of(reply: &Value) -> Value {
    reply
        .get("result")
        .unwrap_or_else(|| &reply["error"]["data"]["code"])
        .clone()
}

/// What one attempt of the seed stages: an enqueue of `/b`, a seed `/s`
/// added through its `ctx.run`, the key `progress` set to the attempt's
/// number and the key `first` set then deleted; and what reading the key
/// `progress` through the attempt answers once it is set.
fn stage(inside: &mut Inside<'_>, handle: &Value) -> Vec<Value> {
    let run_id = &handle["run_id"];
    let attempt = attempt_of(handle);
    let enqueued = inside.enqueue(handle, "https://shop.test/b", "page", None);
    let seeded = inside.call(
        "run.add_seed",
        &json!({
            "run_id": run_id,
            "seed": { "url": "https://shop.test/s", "label": "page" },
            "attempt": attempt,
        }),
    );
    let set = inside.call(
        "key_value_store.set",
        &json!({ "run_id": run_id, "key": "progress", "value": handle["attempt"], "attempt": attempt }),
    );
    let read = inside.call(
        "key_value_store.get",
        &json!({ "run_id": run_id, "key": "progress", "attempt": attempt }),
    );
    inside.call(
        "key_value_store.set",
        &json!({ "run_id": run_id, "key": "first", "value": true, "attempt": attempt }),
    );
    let deleted = inside.call(
        "key_value_store.delete",
        &json!({ "run_id": run_id, "key": "first", "attempt": attempt }),
    );
    vec![
        enqueued,
        answer_of(&seeded),
        answer_of(&set),
        answer_of(&read),
        answer_of(&deleted),
    ]
}

/// The URL of each work item the run file holds, in order.
pub(crate) fn work_items(env: &TestEnv, run_id: &str) -> Vec<String> {
    run_file(env, run_id)
        .prepare("SELECT url FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Each key the run file's Key-value store holds, with its value.
pub(crate) fn keys(env: &TestEnv, run_id: &str) -> Vec<(String, String)> {
    run_file(env, run_id)
        .prepare("SELECT key, value FROM key_values ORDER BY key")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub(crate) fn run_file(env: &TestEnv, run_id: &str) -> Connection {
    Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap()
}

/// The handler's answer that commits one Record naming its page.
fn record_the_page(handle: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "pages", "data": { "page": handle["crawl_request"]["url"] } }] } })
}

/// A crawl conformance case (ADR 0093): the first attempt of the seed
/// enqueues a link, adds a seed and writes keys through its context, then
/// fails retryably; none of it stays, so the retried attempt's enqueue and
/// seed are added again, each fetched once, and its keys are the ones the
/// run file holds. A read through the attempt sees what it staged.
///
/// Covers: story:6-236, operation:crawl_request.enqueue, operation:run.add_seed, operation:key_value_store.set, operation:key_value_store.get, operation:key_value_store.delete
#[test]
fn a_failed_attempts_enqueues_seeds_and_keys_are_discarded_and_its_retry_commits_its_own() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let mut staged = Vec::new();
    let mut handled = Vec::new();

    crawl_under(
        &mut engine,
        "retried",
        &[json!({ "url": SEED, "label": "page" })],
        &json!({ "retry": { "max": 2 } }),
        &mut |inside, handle| {
            handled.push(handle["crawl_request"]["url"].clone());
            if handle["crawl_request"]["url"] != SEED {
                return record_the_page(handle);
            }
            staged.push(stage(inside, handle));
            if handle["attempt"] == 1 {
                json!({ "error": { "code": -32003, "message": "try again", "data": { "code": "retry_requested", "retryable": true } } })
            } else {
                record_the_page(handle)
            }
        },
    );
    assert!(engine.finished());

    let deleted = json!({ "existed": true });
    assert_eq!(
        (
            staged,
            handled,
            work_items(&env, "retried"),
            keys(&env, "retried")
        ),
        (
            vec![
                vec![
                    added("2", "https://shop.test/b"),
                    added("3", "https://shop.test/s"),
                    json!({}),
                    json!({ "value": 1 }),
                    deleted.clone(),
                ],
                vec![
                    added("4", "https://shop.test/b"),
                    added("5", "https://shop.test/s"),
                    json!({}),
                    json!({ "value": 2 }),
                    deleted,
                ],
            ],
            vec![
                json!(SEED),
                json!(SEED),
                json!("https://shop.test/b"),
                json!("https://shop.test/s"),
            ],
            vec![
                SEED.to_owned(),
                "https://shop.test/b".to_owned(),
                "https://shop.test/s".to_owned(),
            ],
            vec![("progress".to_owned(), "2".to_owned())],
        )
    );
}

/// Base64 of 1 MiB and 2 bytes of `a`, past what a key's row holds inline,
/// so the store keeps them as a file of the Run's folder.
fn kept_bytes() -> String {
    "YWFh".repeat(349_526)
}

/// A crawl conformance case (ADR 0093): Key-value store bytes written
/// through a handler's `ctx.run`, inline or kept as a file, are staged with
/// the attempt as its JSON values are: a read through the attempt sees
/// them, the failed attempt leaves neither its row nor its kept file, and
/// the retried attempt's bytes are the ones the run file holds.
///
/// Covers: story:6-236, operation:key_value_store.set_bytes, operation:key_value_store.get_bytes
#[test]
fn a_failed_attempts_bytes_inline_or_kept_are_discarded_and_its_retry_commits_its_own() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let mut read = Vec::new();

    crawl_under(
        &mut engine,
        "bytes",
        &[json!({ "url": SEED, "label": "page" })],
        &json!({ "retry": { "max": 2 } }),
        &mut |inside, handle| {
            let (run_id, attempt) = (&handle["run_id"], attempt_of(handle));
            let small = if handle["attempt"] == 1 {
                "Zmlyc3Q="
            } else {
                "c2Vjb25k"
            };
            inside.call(
                "key_value_store.set_bytes",
                &json!({ "run_id": run_id, "key": "small", "content": small, "content_type": "text/plain", "attempt": attempt }),
            );
            if handle["attempt"] == 1 {
                inside.call(
                    "key_value_store.set_bytes",
                    &json!({ "run_id": run_id, "key": "big", "content": kept_bytes(), "content_type": "text/plain", "attempt": attempt }),
                );
            }
            let got = inside.call(
                "key_value_store.get_bytes",
                &json!({ "run_id": run_id, "key": "small", "attempt": attempt }),
            );
            read.push(answer_of(&got)["bytes"]["content"].clone());
            if handle["attempt"] == 1 {
                json!({ "error": { "code": -32003, "message": "try again", "data": { "code": "retry_requested", "retryable": true } } })
            } else {
                record_the_page(handle)
            }
        },
    );
    assert!(engine.finished());

    let rows: Vec<(String, Option<Vec<u8>>, Option<i64>)> = run_file(&env, "bytes")
        .prepare("SELECT key, content, file FROM key_values ORDER BY key")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let kept = HomeLayout::of(env.home()).run_folder("bytes").join("files");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test lists the run folder's kept files on its own thread, never an async worker"
    )]
    let files = std::fs::read_dir(&kept).map_or(0, Iterator::count);
    assert_eq!(
        (read, rows, files),
        (
            vec![json!("Zmlyc3Q="), json!("c2Vjb25k")],
            vec![("small".to_owned(), Some(b"second".to_vec()), None)],
            0
        )
    );
}

/// A crawl conformance case (ADR 0093): an attempt that fails for good,
/// with its handler's own failure, leaves none of what it staged: no work
/// item but its own, no key; the Records it never answered never commit.
///
/// Covers: story:6-236, operation:crawl_request.enqueue, operation:run.add_seed, operation:key_value_store.set
#[test]
fn an_attempt_that_fails_for_good_leaves_no_link_seed_or_key() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let mut handled = 0;

    crawl_under(
        &mut engine,
        "failed",
        &[json!({ "url": SEED, "label": "page" })],
        &json!({}),
        &mut |inside, handle| {
            handled += 1;
            stage(inside, handle);
            json!({ "error": { "code": -32003, "message": "my bug", "data": { "code": "handler_failed", "retryable": false } } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (handled, work_items(&env, "failed"), keys(&env, "failed")),
        (1, vec![SEED.to_owned()], Vec::new())
    );
}

/// A crawl conformance case (ADR 0093): under `fetch.tier: auto`, the
/// script-rendered feed's attempt runs its handler on `http`, then again in
/// the browser, as a sample, where `http` cannot serve its wait; what the
/// `http` run staged is discarded with its answer, and the browser run's
/// commit is the one that counts.
///
/// Covers: story:6-236, operation:crawl_request.enqueue, operation:locator.wait_for_count
#[test]
fn an_escalated_attempts_first_run_leaves_none_of_what_it_staged() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 869).unwrap();
    let mut engine = crate::fetch_tiers::engine(&env);
    let feed = proving_ground.url("rendered.test", "/feed");

    let mut runs = Vec::new();
    crawl_declaring(
        &mut engine,
        "escalated",
        &[json!({ "url": feed, "label": "page" })],
        &json!({ "page": {}, "child": {} }),
        &crate::fetch_tiers::settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            if handle["crawl_request"]["label"] != "page" {
                return json!({ "result": { "records": [] } });
            }
            let tier = handle["tier"].as_str().unwrap().to_owned();
            let enqueued = inside.enqueue(handle, &format!("/feed?from={tier}"), "child", None);
            runs.push((tier, enqueued));
            let waited = inside.call(
                "locator.wait_for_count",
                &json!({
                    "locator": { "page_id": handle["page_id"], "selector": "li.item" },
                    "count": { "min": 1 },
                }),
            );
            match waited.get("error") {
                Some(error) => json!({ "error": error }),
                None => json!({ "result": { "records": [] } }),
            }
        },
    );
    assert!(engine.finished());

    let from = |tier: &str| proving_ground.url("rendered.test", &format!("/feed?from={tier}"));
    assert_eq!(
        (runs, work_items(&env, "escalated")),
        (
            vec![
                ("http".to_owned(), added("2", &from("http"))),
                ("browser".to_owned(), added("3", &from("browser"))),
            ],
            vec![feed.clone(), from("browser")],
        )
    );
}
