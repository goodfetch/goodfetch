//! What a failed test leaves of its Observation log (ADR 0060): the log as
//! NDJSON in its kept folder, with security values redacted.

use std::path::Path;

use goodfetch_test_support::{LOG_FILE, TestEnv};
use serde_json::json;

use crate::probe;

#[test]
fn a_failed_test_writes_its_observation_log_in_its_kept_folder() {
    let env = TestEnv::new();

    let output = probe(&env, "probe::fails_with_an_observation_log")
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let listed = |prefix: &str| {
        stderr
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .map_or_else(|| panic!("no {prefix} line: {stderr}"), Path::new)
    };
    let path = listed("goodfetch-observation-log=");
    let written = std::fs::read_to_string(path).unwrap();
    let lines: Vec<serde_json::Value> = written
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!output.status.success());
    assert_eq!(path, listed("goodfetch-test-folder-kept=").join(LOG_FILE));
    assert_eq!(
        lines,
        [
            json!({ "at": null, "host": "engine", "operation": { "fence": { "epoch": 1 } } }),
            json!({
                "at": null,
                "host": "client",
                "operation": { "other": { "token": "[redacted]" } },
            }),
        ]
    );
    assert!(!written.contains("gf_token_5f1e"), "{written}");
}
