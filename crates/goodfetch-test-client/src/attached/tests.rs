#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads its home's user configuration, outside any async worker"
)]

use goodfetch_test_support::TestEnv;

use super::*;
use crate::home::HomeLayout;

/// The `log.level` line of the user configuration of `env`'s home.
fn level_of(env: &TestEnv) -> String {
    let file = HomeLayout::of(env.home()).user_configuration();
    let written = std::fs::read_to_string(file).unwrap();
    written
        .lines()
        .find(|line| line.starts_with("log.level"))
        .unwrap()
        .to_owned()
}

/// Whatever command a test starts the engine through, its stdout is read at
/// the level that makes a stray log line show: a caller cannot forget it.
#[test]
fn a_command_started_for_a_test_logs_at_trace() {
    let env = TestEnv::new();
    let mut command = env.process_command("true");

    let engine = Attached::spawned(&env, MESSAGE_DEADLINE, &mut command);
    let _ = engine.finished();

    assert_eq!(level_of(&env), r#"log.level = "trace""#);
}

#[test]
fn a_level_a_test_asks_for_stays() {
    let env = TestEnv::new();

    let engine = Attached::start_logging(&env, "true", MESSAGE_DEADLINE, "warn", |_| {});
    let _ = engine.finished();

    assert_eq!(level_of(&env), r#"log.level = "warn""#);
}

/// An engine a test drops while it runs ends at the end of its input, and
/// the drop waits for it to exit: the test's teardown, which fails a test
/// whose process is still alive (ADR 0069), finds it gone, where under load
/// it once found an engine that took a moment to close (issue 2805).
#[cfg(unix)]
#[test]
fn the_teardown_finds_an_engine_dropped_while_it_runs_gone() {
    let env = TestEnv::new();
    // An engine that closes a moment after its input ends.
    let mut command = env.process_command("sh");
    command.args(["-c", "cat > /dev/null; exec /bin/sleep 0.5"]);
    let engine = Attached::spawned(&env, MESSAGE_DEADLINE, &mut command);

    drop(engine);

    // The teardown panics, failing the test, while a process it recorded
    // still runs.
    drop(env);
}
