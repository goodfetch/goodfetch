//! What a `TestEnv`'s teardown does: it keeps and lists the folder of a
//! failed test, fails a test that left a process alive (ADR 0069), and fails
//! a test whose crash-dump folder holds a dump (ADR 0074).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use goodfetch_test_support::{
    CRASH_DUMP_VARIABLE, CrashDumps, ProcessesAlive, TOOLS_VARIABLE, TestEnv,
};

use crate::{TEST_ENV, manifest, probe, shell};

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
fn a_dump_a_child_writes_in_the_crash_dump_folder_fails_teardown_naming_it() {
    let mut folder = None;

    let failure = catch_unwind(AssertUnwindSafe(|| {
        let env = TestEnv::new();
        folder = Some(env.folder().to_owned());
        let child = shell(
            &env,
            &format!("echo dump > \"${CRASH_DUMP_VARIABLE}/renderer.dmp\""),
            &format!("echo dump> \"%{CRASH_DUMP_VARIABLE}%\\renderer.dmp\""),
        );
        assert!(child.close_stdin_and_wait().unwrap().success());
        drop(env);
    }))
    .unwrap_err();

    let folder = folder.unwrap();
    let dump = folder.join("home").join("crash-dumps").join("renderer.dmp");
    let kept = dump.is_file();
    std::fs::remove_dir_all(&folder).unwrap();
    let dumps = failure.downcast::<CrashDumps>().unwrap();
    assert_eq!(dumps.dumps, [dump]);
    assert!(kept, "the failed test's folder keeps its dump");
}

#[test]
fn a_dump_in_a_subfolder_of_the_crash_dump_folder_fails_teardown() {
    let mut folder = None;

    let failure = catch_unwind(AssertUnwindSafe(|| {
        let env = TestEnv::new();
        folder = Some(env.folder().to_owned());
        let completed = env.crash_dump_folder().join("completed");
        std::fs::create_dir(&completed).unwrap();
        std::fs::write(completed.join("renderer.dmp"), "dump").unwrap();
        drop(env);
    }))
    .unwrap_err();

    let folder = folder.unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    let dumps = failure.downcast::<CrashDumps>().unwrap();
    assert_eq!(
        dumps.dumps,
        [folder
            .join("home")
            .join("crash-dumps")
            .join("completed")
            .join("renderer.dmp")]
    );
}

#[test]
fn an_empty_crash_dump_folder_passes_teardown() {
    let env = TestEnv::new();
    let child = shell(&env, "exit 0", "exit 0");
    child.close_stdin_and_wait().unwrap();
    std::fs::create_dir(env.crash_dump_folder().join("completed")).unwrap();

    let teardown = catch_unwind(AssertUnwindSafe(|| drop(env)));

    assert!(teardown.is_ok());
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
