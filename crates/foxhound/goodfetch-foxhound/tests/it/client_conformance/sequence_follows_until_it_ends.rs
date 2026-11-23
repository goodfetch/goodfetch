//! A sequence's command run with `--follow` prints what the call sends as
//! it arrives, one JSON object per line under `--json`, and ends when the
//! sequence ends (ADR 0099), through Foxhound's executable: `run records`
//! of a code-free crawl that ended sends each Record its Dataset holds,
//! one per credit the CLI grants, then answers.

use goodfetch_test_support::{ProvingGround, TestEnv, cli};
use serde_json::json;

use crate::client_conformance::FOXHOUND;
use crate::crawl_commands::{crawl_start, settings_file, short_idle, start_home_daemon};

/// Covers: story:11-198, story:11-1131, command:run.records
#[test]
fn attached() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = start_home_daemon(&env);
    let proving_ground = ProvingGround::start(&env, 1512).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url("extraction/7").unwrap();
    crawl_start(&env, &settings, "followed", std::slice::from_ref(&index));
    // The crawl has ended once its last ending is read.
    let _ = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "followed", "--follow", "--json"])
        .run()
        .unwrap();

    let checked = cli(&env, FOXHOUND)
        .args([
            "run",
            "records",
            "followed",
            "--dataset",
            "pages",
            "--follow",
            "--json",
            "--attached",
        ])
        .json_lines("RecordRead")
        .unwrap();

    let read: Vec<_> = checked
        .documents
        .iter()
        .map(|read| {
            (
                read["record"]["number"].clone(),
                read["record"]["url"].clone(),
            )
        })
        .collect();
    assert_eq!(
        (checked.output.exit, read),
        (Some(0), vec![(json!(1), json!(index))])
    );
}
