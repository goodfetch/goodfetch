use goodfetch_test_support::{RuntimeFile, TestEnv};

use super::*;

#[test]
fn the_per_major_folder_holds_the_user_configuration_test_support_writes() {
    let env = TestEnv::new();

    let layout = HomeLayout::of(env.home());

    assert!(layout.user_configuration().is_file());
}

#[test]
fn the_runtime_file_is_where_test_support_reads_a_daemons() {
    let env = TestEnv::new();

    let layout = HomeLayout::of(env.home());

    assert_eq!(
        layout.daemon_file(DaemonFile::RuntimeFile),
        RuntimeFile::path(env.home(), product_version().as_str())
    );
}

#[test]
fn a_runs_file_and_trace_sit_in_its_run_folder_under_the_per_major_folder() {
    let layout = HomeLayout::of(Path::new("home"));

    let paths = (
        layout.run_file("first"),
        layout.run_trace("first", home::RUN_TRACE_PROVENANCE),
    );

    assert_eq!(
        paths,
        (
            layout.per_major().join("runs/first/run.sqlite"),
            layout.per_major().join("runs/first/trace/provenance.jsonl"),
        )
    );
}
