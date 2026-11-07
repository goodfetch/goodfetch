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
