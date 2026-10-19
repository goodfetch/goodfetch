//! What a `TestEnv`'s teardown does: it keeps and lists the folder of a
//! failed test, and fails a test that left a process alive (ADR 0069).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use goodfetch_test_support::{ProcessesAlive, TOOLS_VARIABLE, TestEnv};

use crate::{TEST_ENV, manifest, probe};

#[test]
fn a_kept_folder_is_listed_on_failure() {
    let env = TestEnv::new();

    let output = probe(&env, "probe::fails_with_a_test_env")
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let kept = stderr
        .lines()
        .find_map(|line| line.strip_prefix("goodfetch-test-folder-kept="))
        .unwrap_or_else(|| panic!("no kept folder listed: {stderr}"));
    assert!(!output.status.success());
    assert!(Path::new(kept).is_dir(), "{kept}");
    assert!(Path::new(kept).starts_with(env.folder()), "{kept}");
}

#[test]
fn a_child_left_alive_after_teardown_fails_the_test() {
    let outer = TestEnv::new();
    let manifest = manifest(&outer);
    let mut folder = None;

    let failure = catch_unwind(AssertUnwindSafe(|| {
        let env = TestEnv::new();
        folder = Some(env.folder().to_owned());
        let mut command = env.process_command(TEST_ENV);
        command.env(TOOLS_VARIABLE, &manifest);
        env.spawn(&mut command).unwrap();
        drop(env);
    }))
    .unwrap_err();

    std::fs::remove_dir_all(folder.unwrap()).unwrap();
    let alive = failure.downcast::<ProcessesAlive>().unwrap();
    assert_eq!(alive.programs, ["goodfetch-test-env"]);
}

#[test]
fn a_child_that_exited_passes_teardown() {
    let env = TestEnv::new();
    let mut command = env.process_command(TEST_ENV);
    command.env(TOOLS_VARIABLE, manifest(&env));
    let process = env.spawn(&mut command).unwrap();
    process.close_stdin_and_wait().unwrap();

    let teardown = catch_unwind(AssertUnwindSafe(|| drop(env)));

    assert!(teardown.is_ok());
}
