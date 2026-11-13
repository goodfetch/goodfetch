//! An attempt whose outcome the run file refused to write (issue 2320):
//! its work item is retried, never left leased under a Run that finishes.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, page_of, records, run_file, seeds};

/// The first attempt's Records are refused by the run file, through a
/// trigger the test plants while its handler holds the attempt: no
/// storage failure, so the Run goes on. The attempt ends as one whose
/// outcome was not written and is retried, its second attempt commits,
/// and the Run finishes complete with no request pending.
///
/// Covers: story:3-41
#[test]
fn an_attempt_whose_outcome_was_not_written_is_retried_and_the_run_finishes_complete() {
    let env = TestEnv::new();
    let mut attempts = Vec::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "lost", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if attempts.is_empty() {
                    let file = run_file(&env, "lost");
                    file.busy_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    file.execute_batch(
                        "CREATE TRIGGER refuse_the_first_attempts_records BEFORE INSERT ON records
                         WHEN (SELECT attempts FROM work_items WHERE id = NEW.work_item) = 1
                         BEGIN SELECT RAISE(ABORT, 'refused by the test'); END",
                    )
                    .unwrap();
                }
                attempts.push(page_of(params));
                Reply::Records(vec![json!({ "page": page_of(params) })])
            };
            client
                .call("run.finish", json!({ "run_id": "lost" }), &mut handler)
                .await
        },
    );

    let file = run_file(&env, "lost");
    let leased: i64 = file
        .query_row(
            "SELECT COUNT(*) FROM work_items WHERE status = 'leased'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (
            attempts.len(),
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["requests"]["succeeded"],
            &report["result"]["requests"]["pending"],
            records(&file).len(),
            leased,
        ),
        (
            2,
            &json!("finished"),
            &json!("complete"),
            &json!(1),
            &json!(0),
            1,
            0
        ),
        "{report}"
    );
}
