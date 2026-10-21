//! The commands generated from the catalog, through Collie's executable on
//! the Attached engine it runs in its own process (ADRs 0024, 0069 and
//! 0099): `<namespace> <member>`, a value's ID first and `--<parameter>`
//! flags; an opener prints the new value's ID, a round trip its result;
//! `--json` leaves one JSON value on stdout, the result or `{ "error": … }`;
//! warnings and failures go to stderr; a failure exits with its group's
//! row. One snapshot per command holds its exit code, stdout and stderr.

use goodfetch_test_support::{
    SnapshotValues, TestEnv, assert_cli_snapshot, cli, snapshot_settings,
};
use serde_json::json;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Starts the Run `run_id` with `run start`, on an Attached engine that
/// stops it as the command ends.
fn start(env: &TestEnv, run_id: &str) {
    let output = cli(env, COLLIE)
        .args(["run", "start", "--run-id", run_id])
        .run()
        .unwrap();
    assert_eq!(output.exit, Some(0), "{output}");
}

/// Covers: story:11-197, command:engine.info
#[test]
fn engine_info_prints_the_engines_product_and_versions_one_field_per_line() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE).args(["engine", "info"]).run().unwrap();

    snapshot_settings(
        &SnapshotValues::of(&env).with_executable(env!("CARGO_PKG_VERSION"), "no-commit"),
    )
    .bind(|| {
        assert_cli_snapshot!(output, @r#"
        exit: 0
        ----- stdout -----
        product: collie
        product_major: 0
        product_version: [PRODUCT_VERSION]
        protocol_version: 0.1
        ----- stderr -----
        [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
        [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="engine.info"
        "#);
    });
}

/// Covers: story:11-204, command:engine.info
#[test]
fn engine_info_json_prints_the_engine_info_and_nothing_else() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["engine", "info", "--json"])
        .json("EngineInfo")
        .unwrap();

    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    assert_eq!(checked.documents[0]["product"], json!("collie"));
}

/// Covers: story:11-197, command:run.start
#[test]
fn run_start_prints_the_new_runs_id() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @r#"
    exit: 0
    ----- stdout -----
    first
    ----- stderr -----
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="run.start"
    "#);
}

/// Covers: story:11-204, command:run.start
#[test]
fn run_start_json_prints_the_run_started() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first", "--json"])
        .json("RunStarted")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "lifecycle": "running",
      "needs": "collie",
      "run_id": "first",
      "started_as": "new"
    }
    ----- stderr -----
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="run.start"
    "#);
}

/// Covers: story:11-197, story:11-204, command:run.finish
#[test]
fn run_finish_reads_the_report_of_the_run_the_attached_engine_stopped() {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--json"])
        .json("RunReport")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "completeness": "incomplete",
      "ended_at": "[INSTANT]",
      "lifecycle": "stopped",
      "run_id": "first",
      "started_at": "[INSTANT]",
      "stop_reason": {
        "by": "client",
        "kind": "abort"
      },
      "verdict": "verified"
    }
    ----- stderr -----
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="run.finish"
    "#);
}

/// Covers: command:run.abort
#[test]
fn run_abort_prints_the_report_of_an_ended_run_one_field_per_line() {
    let env = TestEnv::new();
    start(&env, "first");

    let output = cli(&env, COLLIE)
        .args(["run", "abort", "first"])
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @r#"
    exit: 0
    ----- stdout -----
    completeness: incomplete
    ended_at: [INSTANT]
    lifecycle: stopped
    run_id: first
    started_at: [INSTANT]
    stop_reason: {"by":"client","kind":"abort"}
    verdict: verified
    ----- stderr -----
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="run.abort"
    "#);
}

/// Covers: story:11-204, command:sample.echo
#[test]
fn a_deprecated_commands_warning_reaches_stderr_and_never_stdout() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["sample", "echo", "--text", "hello", "--json"])
        .json("SampleEchoResult")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "text": "hello"
    }
    ----- stderr -----
    warning: `sample echo` is deprecated since 0.1: A sample entry, which the real catalog replaces.
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="sample.echo"
    "#);
}

/// Covers: story:11-204, command:run.finish
#[test]
fn a_failed_call_under_json_prints_the_error_with_its_code_group_and_operation() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["run", "finish", "missing", "--json"])
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["error"]["code"],
            &checked.documents[0]["error"]["group"],
            &checked.documents[0]["error"]["operation"],
        ),
        (
            Some(3),
            &json!("run_not_found"),
            &json!("program"),
            &json!("run.finish")
        )
    );
    assert_cli_snapshot!(checked.output);
}

/// Covers: command:run.finish
#[test]
fn a_failed_call_prints_its_message_code_and_retryability_on_stderr() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["run", "finish", "missing"])
        .run()
        .unwrap();

    assert_eq!((output.exit, output.stdout.as_str()), (Some(3), ""));
    assert_cli_snapshot!(output);
}
