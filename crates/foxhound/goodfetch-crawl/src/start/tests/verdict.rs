//! A crawl's Verdict (ADR 0029), through the engine's public interface over
//! the in-memory network: Block classification switched off is a check the
//! crawl's results rely on, so its Run ends `needs_review` with a reason
//! naming `fetch.block_classification`; a control switched off that removes
//! no check on the results, such as `provenance.log`, is listed in
//! `weakened` only; with nothing switched off the crawl is `verified`; and
//! a finished crawl that returns to `running` and ends again is given its
//! Verdict anew (ADR 0102).

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, answer_all, crawl, dependencies, seeds};

/// The report of the crawl `run_id` of two seeds, started with `settings`,
/// once `run.finish` drained it.
fn finished_with(run_id: &str, settings: &Value) -> Value {
    let env = TestEnv::new();
    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": run_id, "seeds": seeds(2), "settings": settings, "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": run_id }), &mut answer_all)
                .await
        },
    );
    report["result"].clone()
}

/// The keys of a report's `weakened` list.
fn weakened(report: &Value) -> Vec<&str> {
    report["weakened"]
        .as_array()
        .map(|listed| {
            listed
                .iter()
                .filter_map(|setting| setting["key"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// The codes of a report's `verdict_reasons`.
fn reasons(report: &Value) -> Vec<&str> {
    report["verdict_reasons"]
        .as_array()
        .map(|given| {
            given
                .iter()
                .filter_map(|reason| reason["code"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Covers: story:11-128
#[test]
fn a_crawl_with_block_classification_off_needs_review_naming_the_key() {
    let report = finished_with(
        "unclassified",
        &json!({ "fetch": { "block_classification": false } }),
    );

    assert_eq!(
        (
            &report["lifecycle"],
            &report["verdict"],
            reasons(&report),
            weakened(&report),
        ),
        (
            &json!("finished"),
            &json!("needs_review"),
            vec!["fetch.block_classification"],
            vec!["fetch.block_classification"],
        ),
        "{report}"
    );
}

/// Covers: story:11-128
#[test]
fn a_crawl_with_only_provenance_off_is_verified_with_the_key_in_weakened() {
    let report = finished_with("unlogged", &json!({ "provenance": { "log": false } }));

    assert_eq!(
        (&report["verdict"], reasons(&report), weakened(&report)),
        (
            &json!("verified"),
            Vec::<&str>::new(),
            vec!["provenance.log"]
        ),
        "{report}"
    );
}

/// Covers: story:11-128
#[test]
fn a_crawl_with_nothing_switched_off_is_verified_with_no_reason() {
    let report = finished_with("checked", &json!({}));

    assert_eq!(
        (
            &report["verdict"],
            &report["verdict_reasons"],
            weakened(&report)
        ),
        (&json!("verified"), &Value::Null, Vec::<&str>::new()),
        "{report}"
    );
}

/// A crawl that ended `verified`, whose failed request is retried with
/// Block classification switched off: the Run returns to `running`, and
/// its next end gives the Verdict anew, `needs_review` naming the key.
///
/// Covers: story:11-128, story:6-306, operation:run.retry_failed
#[test]
fn a_retried_crawl_is_given_its_verdict_anew_when_it_ends_again() {
    let env = TestEnv::new();

    let (first, retried, second) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "again", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let first = client
                .call("run.finish", json!({ "run_id": "again" }), &mut |_| {
                    Reply::Fails("handler_failed", false)
                })
                .await;
            let retried = client
                .call(
                    "run.retry_failed",
                    json!({
                        "run_id": "again",
                        "settings": { "fetch": { "block_classification": false } },
                    }),
                    &mut answer_all,
                )
                .await;
            let second = client
                .call("run.finish", json!({ "run_id": "again" }), &mut answer_all)
                .await;
            (first, retried, second)
        },
    );

    assert_eq!(
        (
            &first["result"]["verdict"],
            &first["result"]["verdict_reasons"],
            &retried["result"],
            &second["result"]["lifecycle"],
            &second["result"]["verdict"],
            reasons(&second["result"]),
        ),
        (
            &json!("verified"),
            &Value::Null,
            &json!({ "readmitted": 1 }),
            &json!("finished"),
            &json!("needs_review"),
            vec!["fetch.block_classification"],
        ),
        "{first}\n{second}"
    );
}

/// Bumps the `run` row's epoch on disk so the live engine's `write_last`
/// fails with `RunStateError::Fenced` (a non-storage error -> `Ended::Unwritten`).
#[expect(
    clippy::disallowed_methods,
    reason = "test helper locating and fencing the run file on disk"
)]
fn fence_run_file(root: &std::path::Path) {
    let mut stack = vec![root.to_path_buf()];
    let mut found = None;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some("run.sqlite") {
                found = Some(path);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let run_file = found.expect("crawl.start creates run.sqlite");
    let conn = rusqlite::Connection::open(&run_file).unwrap();
    conn.execute("UPDATE run SET epoch = epoch + 1", [])
        .unwrap();
}

/// Issue 2549: a crawl with `fetch.block_classification: false` whose end
/// cannot be written (a non-storage error) still reports `needs_review` with
/// `fetch.block_classification` and `end_not_written`, never `verified`.
#[test]
fn an_unwritten_crawl_with_block_classification_off_needs_review_naming_the_key() {
    let env = TestEnv::new();
    let home = env.home().to_owned();
    let reply = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "unwritten-unclassified",
                        "seeds": seeds(1),
                        "settings": {
                            "run_state": { "durability": "full" },
                            "fetch": { "block_classification": false },
                        },
                        "labels": { "page": {} },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            fence_run_file(&home);
            client
                .call(
                    "run.stop",
                    json!({ "run_id": "unwritten-unclassified" }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );
    let report = &reply["result"];
    assert_eq!(
        (&report["verdict"], reasons(report), weakened(report)),
        (
            &json!("needs_review"),
            vec!["fetch.block_classification", "end_not_written"],
            vec!["fetch.block_classification"],
        ),
        "{report}"
    );
}

/// Issue 2549: a crawl with nothing switched off whose end cannot be written
/// does not report `verified`; its reason says the end was not written.
#[test]
fn an_unwritten_crawl_with_nothing_switched_off_does_not_claim_verified() {
    let env = TestEnv::new();
    let home = env.home().to_owned();
    let reply = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "unwritten-checked",
                        "seeds": seeds(1),
                        "settings": { "run_state": { "durability": "full" } },
                        "labels": { "page": {} },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            fence_run_file(&home);
            client
                .call(
                    "run.stop",
                    json!({ "run_id": "unwritten-checked" }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );
    let report = &reply["result"];
    assert_eq!(
        (&report["verdict"], reasons(report)),
        (&json!("needs_review"), vec!["end_not_written"]),
        "{report}"
    );
}
