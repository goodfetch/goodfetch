//! The seed line a test prints when its `TestEnv` starts (ADR 0060).

use std::io::{BufRead as _, BufReader};
use std::process::Stdio;

use goodfetch_test_support::TestEnv;

use crate::probe;

/// The probe sleeps after creating its `TestEnv`, as a test nextest kills on
/// a timeout does: the line must already be on its stderr.
#[test]
fn seed_line_is_printed_before_the_test_body_runs() {
    let env = TestEnv::new();
    let mut command = probe(&env, "probe::hangs_after_creating_a_test_env");
    command
        .env("GOODFETCH_TEST_SEED", "4242")
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let stderr = process.child().stderr.take().unwrap();

    let line = BufReader::new(stderr)
        .lines()
        .map(Result::unwrap)
        .find(|line| line.starts_with("goodfetch-test-seed="));

    process.stop().unwrap();
    assert_eq!(line.as_deref(), Some("goodfetch-test-seed=4242"));
}
