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

/// A failed in-memory test's folder is kept in the temporary folder on
/// disk, not on the memory file system, where every kept folder would hold
/// RAM until someone deletes it (TESTING.md, Shared helpers).
#[cfg(target_os = "linux")]
#[test]
fn an_in_memory_folder_of_a_failed_test_is_kept_on_disk() {
    let env = TestEnv::new();

    let output = probe(&env, "probe::fails_with_an_in_memory_test_env")
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let kept = stderr
        .lines()
        .find_map(|line| line.strip_prefix("goodfetch-test-folder-kept="))
        .map_or_else(|| panic!("no kept folder listed: {stderr}"), Path::new);
    let name = kept.file_name().unwrap();
    let left_in_memory = Path::new("/dev/shm").join(name);
    let still_there = left_in_memory.exists();
    if still_there {
        std::fs::remove_dir_all(&left_in_memory).unwrap();
    }
    assert!(!output.status.success());
    assert!(
        !still_there,
        "{} is left in memory",
        left_in_memory.display()
    );
    assert!(kept.starts_with(env.folder()), "{}", kept.display());
    assert_eq!(
        std::fs::read_to_string(kept.join("evidence.txt")).unwrap(),
        "kept"
    );
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

/// Crashpad's database, as every Chromium that ran with the folder writes
/// it, crash or no crash: its settings and its empty report folders.
#[test]
fn the_crash_reporters_own_database_passes_teardown() {
    let env = TestEnv::new();
    let folder = env.crash_dump_folder();
    std::fs::write(folder.join("settings.dat"), "settings").unwrap();
    for report_folder in ["new", "pending", "completed", "attachments"] {
        std::fs::create_dir(folder.join(report_folder)).unwrap();
    }

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

#[cfg(unix)]
#[test]
fn the_socket_folder_a_killed_daemon_left_outside_the_home_goes_at_teardown() {
    use goodfetch_test_support::{ConsoleState, DAEMON_SOCKET, DaemonEndpoint, RuntimeFile};

    let outside = tempfile::Builder::new()
        .prefix("goodfetch é ")
        .tempdir_in("/tmp")
        .unwrap()
        .keep();
    let env = TestEnv::new();
    let runtime_file = RuntimeFile::path(env.home(), "0.0.0");
    std::fs::create_dir_all(runtime_file.parent().unwrap()).unwrap();
    RuntimeFile {
        endpoint: DaemonEndpoint::Unix(outside.join(DAEMON_SOCKET)),
        token: "a-token".to_owned(),
        version: "0.0.0".to_owned(),
        pid: 1,
        console: ConsoleState::Absent,
    }
    .write(&runtime_file)
    .unwrap();

    drop(env);

    let left = outside.exists();
    if left {
        std::fs::remove_dir_all(&outside).unwrap();
    }
    assert!(!left, "{}", outside.display());
}
