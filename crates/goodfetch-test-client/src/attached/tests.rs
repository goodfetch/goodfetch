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

/// A program that ends once its input does and writes nothing on its stdout:
/// Windows has no `true`, and `findstr` searches its input for its argument.
const QUIET: &str = if cfg!(windows) { "findstr" } else { "true" };

/// Whatever command a test starts the engine through, its stdout is read at
/// the level that makes a stray log line show: a caller cannot forget it.
#[test]
fn a_command_started_for_a_test_logs_at_trace() {
    let env = TestEnv::new();
    let mut command = env.process_command(QUIET);
    command.arg(ATTACHED_ENGINE);

    let engine = Attached::spawned(&env, MESSAGE_DEADLINE, &mut command);
    let _ = engine.finished();

    assert_eq!(level_of(&env), r#"log.level = "trace""#);
}

#[test]
fn a_level_a_test_asks_for_stays() {
    let env = TestEnv::new();

    let engine = Attached::start_logging(&env, QUIET, MESSAGE_DEADLINE, "warn", |_| {});
    let _ = engine.finished();

    assert_eq!(level_of(&env), r#"log.level = "warn""#);
}

/// The lines of the answer record of `env`'s test, none when it has none.
fn answer_record(env: &TestEnv) -> Vec<Value> {
    std::fs::read_to_string(env.folder().join(goodfetch_test_support::ANSWER_RECORD))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// An engine that writes no JSON log, as one refusing its configuration
/// does, leaves the calls it answered in the test's answer record when the
/// test ends it with `finish_unlogged`: the exercise audit reads the
/// refusal there (#1635).
#[cfg(unix)]
#[test]
fn an_unlogged_engine_s_answers_go_to_the_answer_record() {
    ends_a_refusing_engine_with(Attached::finish_unlogged, 1);
}

/// `finish` leaves the calls an engine answered to its log, which holds
/// their dispatch records.
#[cfg(unix)]
#[test]
fn a_finished_engine_s_answers_stay_out_of_the_answer_record() {
    ends_a_refusing_engine_with(Attached::finish, 0);
}

/// Runs a stand-in engine that refuses one call, ends it with `end`, and
/// checks that the answer record holds the refusal `recorded` times.
#[cfg(unix)]
fn ends_a_refusing_engine_with(end: fn(Attached) -> (Vec<Value>, bool), recorded: usize) {
    let env = TestEnv::new();
    // A stand-in that refuses the first call and exits by itself, logging
    // nothing.
    let mut command = env.process_command("sh");
    command.args([
        "-c",
        r#"read call; echo '{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"refused","data":{"code":"setting_invalid","retryable":false}}}'; cat > /dev/null"#,
    ]);
    let mut engine = Attached::spawned(&env, MESSAGE_DEADLINE, &mut command);
    let _ = engine.ask(1, "config.explain", &json!({}));

    let (_, exited) = end(engine);

    assert!(exited);
    let expected = json!({
        "target": goodfetch_test_support::DISPATCH_TARGET,
        "fields": {
            "message": "request failed, as its Client read it",
            "method": "config.explain",
            "error_code": "setting_invalid",
        },
    });
    assert_eq!(answer_record(&env), vec![expected; recorded]);
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
