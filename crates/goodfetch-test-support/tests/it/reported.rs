//! The address a child reports on its first line of output.

use std::time::Duration;

use goodfetch_test_support::{
    Process, ReportedAddressError, TestEnv, reported_address, reported_address_within,
};

/// Starts the OS's own shell on a script: `sh` on Unix, `cmd` on Windows.
fn shell(env: &TestEnv, unix: &str, windows: &str) -> Process {
    let mut command = if cfg!(windows) {
        let mut command = env.process_command(&env.env()["ComSpec"]);
        command.args(["/C", windows]);
        command
    } else {
        let mut command = env.process_command("/bin/sh");
        command.args(["-c", unix]);
        command
    };
    env.spawn(&mut command).unwrap()
}

#[test]
fn reported_address_reads_the_childs_first_line() {
    let env = TestEnv::new();
    let process = shell(
        &env,
        r#"echo '{"address":"127.0.0.1:4000","version":"0.0.0"}'; read line"#,
        r#"echo {"address":"127.0.0.1:4000","version":"0.0.0"}& set /p line="#,
    );

    let reported = reported_address(&mut process.child());

    process.close_stdin_and_wait().unwrap();
    let reported = reported.unwrap();
    assert_eq!(reported.address, "127.0.0.1:4000");
    assert_eq!(reported.line["version"], "0.0.0");
}

#[test]
fn reported_address_fails_at_its_deadline() {
    let env = TestEnv::new();
    let process = shell(&env, "read line", "set /p line=");

    let reported = reported_address_within(&mut process.child(), Duration::from_millis(200));

    process.close_stdin_and_wait().unwrap();
    assert!(
        matches!(reported, Err(ReportedAddressError::Deadline(_))),
        "{reported:?}"
    );
}

#[test]
fn a_first_line_without_an_address_is_refused() {
    let env = TestEnv::new();
    let process = shell(&env, r#"echo '{"port":4000}'"#, r#"echo {"port":4000}"#);

    let reported = reported_address(&mut process.child());

    process.close_stdin_and_wait().unwrap();
    assert!(
        matches!(reported, Err(ReportedAddressError::NoAddress(_))),
        "{reported:?}"
    );
}
