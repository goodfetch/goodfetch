//! Tests that other tests run as child processes, to watch what a test
//! using `TestEnv` shows when it hangs or fails. They are ignored, so they
//! run only as those children.

use std::time::Duration;

use goodfetch_test_support::TestEnv;

#[test]
#[ignore = "run as a child by seed::seed_line_is_printed_before_the_test_body_runs"]
fn hangs_after_creating_a_test_env() {
    let _env = TestEnv::new();

    // The parent kills it long before this ends.
    std::thread::sleep(Duration::from_secs(600));
}

#[test]
#[ignore = "run as a child by teardown::a_kept_folder_is_listed_on_failure"]
fn fails_with_a_test_env() {
    let _env = TestEnv::new();

    panic!("the probe fails on purpose");
}
