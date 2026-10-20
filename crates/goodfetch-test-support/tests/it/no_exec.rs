//! `NoExec` (ADR 0067, Linux): a child under its seccomp filter that starts
//! a program is refused, and one that starts none runs as it would without
//! it. The child is coreutils' `env`, which exits 126 when it finds the
//! program it is asked to start but cannot start it.

use std::io::Write as _;

use goodfetch_test_support::{NoExec, TestEnv, command};

/// `env`'s exit status when it cannot start the program it found.
const CANNOT_START: i32 = 126;

#[test]
fn a_child_under_no_exec_that_starts_a_program_is_refused() {
    let env = TestEnv::new();

    let output = NoExec::new("/usr/bin/env")
        .arg("/bin/true")
        .output(&env)
        .unwrap();

    assert_eq!(output.status.code(), Some(CANNOT_START), "{output:?}");
}

#[test]
fn the_same_child_without_no_exec_starts_the_program() {
    let env = TestEnv::new();

    let output = command(&env, "/usr/bin/env")
        .arg("/bin/true")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn a_child_under_no_exec_that_starts_no_program_runs_normally() {
    let env = TestEnv::new();

    let output = NoExec::new("/bin/echo").arg("ran").output(&env).unwrap();

    assert_eq!(
        (output.status.code(), output.stdout.as_slice()),
        (Some(0), b"ran\n".as_slice())
    );
}

#[test]
fn a_child_under_no_exec_has_the_tests_environment() {
    let env = TestEnv::new();

    let output = NoExec::new("/usr/bin/env").output(&env).unwrap();

    let home = format!("GOODFETCH_HOME={}", env.home().display());
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.lines().any(|line| line == home), "{printed}");
}

#[test]
fn a_spawned_child_under_no_exec_is_recorded_and_reads_its_input() {
    let env = TestEnv::new();
    let process = NoExec::new("/bin/cat").spawn(&env).unwrap();

    process
        .child()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"line\n")
        .unwrap();
    let status = process.close_stdin_and_wait().unwrap();

    assert!(status.success(), "{status:?}");
}

#[test]
fn a_program_that_does_not_exist_fails_to_start() {
    let env = TestEnv::new();

    let started = NoExec::new("/nonexistent/program").output(&env);

    assert_eq!(
        started.map_err(|error| error.kind()).err(),
        Some(std::io::ErrorKind::NotFound)
    );
}
