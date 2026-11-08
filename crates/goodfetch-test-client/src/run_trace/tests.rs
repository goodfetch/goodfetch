use goodfetch_test_support::TestEnv;

use super::*;

#[test]
fn a_robots_txt_read_returns_once_the_runs_provenance_records_one() {
    let env = TestEnv::new();
    let file = HomeLayout::of(env.home()).run_trace("read", RUN_TRACE_PROVENANCE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        "{\"action\":\"page_weakly_checked\"}\n{\"action\":\"robots_txt\"}\n",
    )
    .unwrap();

    robots_txt_read(&env, "read");

    assert_eq!(robots_txt_readings(&env, "read").len(), 1);
}

/// Whether reading the Run `run_id`'s readings fails the test, as a
/// Provenance it cannot read must: what is around the read may panic for its
/// own reasons, so only the read's own panic counts.
fn read_panics(env: &TestEnv, run_id: &str) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        robots_txt_readings(env, run_id)
    }))
    .is_err()
}

#[test]
fn a_provenance_line_that_is_not_json_fails_the_read() {
    let env = TestEnv::new();
    let file = HomeLayout::of(env.home()).run_trace("garbled", RUN_TRACE_PROVENANCE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        "{\"action\":\"robots_txt\"\n{\"action\":\"robots_txt\"}\n",
    )
    .unwrap();

    assert!(read_panics(&env, "garbled"));
}

#[test]
fn a_provenance_file_that_cannot_be_read_fails_the_read() {
    let env = TestEnv::new();
    let file = HomeLayout::of(env.home()).run_trace("unreadable", RUN_TRACE_PROVENANCE);
    std::fs::create_dir_all(file).unwrap();

    assert!(read_panics(&env, "unreadable"));
}

#[test]
fn a_line_still_being_written_is_left_for_the_next_read() {
    let env = TestEnv::new();
    let file = HomeLayout::of(env.home()).run_trace("writing", RUN_TRACE_PROVENANCE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, "{\"action\":\"robots_txt\"}\n{\"action\":\"robo").unwrap();

    let readings = robots_txt_readings(&env, "writing");

    assert_eq!(readings, vec![json!({ "action": "robots_txt" })]);
}

#[test]
fn a_run_with_no_provenance_yet_has_no_reading() {
    let env = TestEnv::new();

    let readings = robots_txt_readings(&env, "not-started");

    assert_eq!(readings, Vec::<Value>::new());
}
