//! The commands generated from the catalog, through Collie's executable
//! (ADRs 0024, 0069 and 0099): `<namespace> <member>`, a value's ID first
//! and `--<parameter>` flags; an opener prints the new value's ID, a round
//! trip its result; `--json` leaves one JSON value on stdout, the result or
//! `{ "error": … }`; warnings and failures go to stderr; a failure exits
//! with its group's row. Each command runs in an rstest case per mode, on
//! an Attached engine in the executable's own process and as a Client of
//! the Daemon, and both cases share one snapshot of its exit code, stdout
//! and stderr (ADR 0069).

use std::fmt::Write as _;
use std::path::PathBuf;

use goodfetch_protocol::home::RUNS;
use goodfetch_test_support::{
    CliOutput, Mode, SnapshotValues, TestEnv, assert_cli_snapshot, cli, snapshot_settings,
};
use rstest::rstest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::clean_up_rules::calendar_free;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Starts the Run `run_id` with `run start --attached`, on an Attached
/// engine that stops it as the command ends.
pub(crate) fn start(env: &TestEnv, run_id: &str) {
    let output = cli(env, COLLIE)
        .args(["run", "start", "--run-id", run_id])
        .mode(Mode::Attached)
        .run()
        .unwrap();
    assert_eq!(output.exit, Some(0), "{output}");
}

/// `output` without the engine's dispatch records, so that both modes share
/// one snapshot of what the command itself prints; and, in a Run report it
/// prints, without the warning that depends on the calendar
/// ([`calendar_free`]).
fn printed(mut output: CliOutput) -> CliOutput {
    output.stdout = calendar_free_report(&output.stdout);
    output.without_dispatch_records()
}

/// `stdout` without the calendar's warning in the Run report it prints,
/// as one JSON document or one field per line; unchanged when the report
/// carries no such warning.
fn calendar_free_report(stdout: &str) -> String {
    if let Ok(mut report) = serde_json::from_str::<Value>(stdout) {
        let Some(warnings) = report.get("warnings").and_then(Value::as_array) else {
            return stdout.to_owned();
        };
        let kept = calendar_free(warnings);
        if kept.len() == warnings.len() {
            return stdout.to_owned();
        }
        let fields = report
            .as_object_mut()
            .expect("invariant: a report with warnings is an object");
        if kept.is_empty() {
            fields.remove("warnings");
        } else {
            fields.insert("warnings".to_owned(), Value::Array(kept));
        }
        return format!("{report:#}\n");
    }
    let mut changed = false;
    let kept = stdout
        .lines()
        .filter_map(|line| {
            let Some(warnings) = line
                .strip_prefix("warnings: ")
                .and_then(|listed| serde_json::from_str::<Vec<Value>>(listed).ok())
            else {
                return Some(line.to_owned());
            };
            let kept = calendar_free(&warnings);
            if kept.len() == warnings.len() {
                return Some(line.to_owned());
            }
            changed = true;
            (!kept.is_empty()).then(|| format!("warnings: {}", Value::Array(kept)))
        })
        .fold(String::new(), |mut kept, line| {
            kept.push_str(&line);
            kept.push('\n');
            kept
        });
    if changed { kept } else { stdout.to_owned() }
}

/// The folder of the Run `run_id` that `run start` made in the test's
/// Goodfetch home: `runs/<run_id>` under the folder of the executable's
/// major.
#[expect(
    clippy::disallowed_methods,
    reason = "a test finds the Run's folder in its own Goodfetch home"
)]
pub(crate) fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    std::fs::read_dir(env.home())
        .unwrap()
        .map(|entry| entry.unwrap().path().join(RUNS).join(run_id))
        .find(|folder| folder.is_dir())
        .unwrap()
}

/// Stores `bytes` as a body of the Run `run_id` the way the engine does, a
/// file of `bodies/` named by their lowercase hex SHA-256, and answers its
/// ID.
#[expect(
    clippy::disallowed_methods,
    reason = "a test lays out a body by hand, as the engine stores one"
)]
pub(crate) fn store_body(env: &TestEnv, run_id: &str, bytes: &[u8]) -> String {
    let id = Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut id, byte| {
            let _ = write!(id, "{byte:02x}");
            id
        });
    let bodies = run_folder(env, run_id).join("bodies");
    std::fs::create_dir_all(&bodies).unwrap();
    std::fs::write(bodies.join(&id), bytes).unwrap();
    id
}

/// Covers: story:11-197, command:engine.info
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn engine_info_prints_the_engines_product_and_versions_one_field_per_line(#[case] mode: Mode) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["engine", "info"])
        .mode(mode)
        .run()
        .unwrap();

    snapshot_settings(
        &SnapshotValues::of(&env).with_executable(env!("CARGO_PKG_VERSION"), "no-commit"),
    )
    .bind(|| {
        assert_cli_snapshot!(printed(output), @r"
        exit: 0
        ----- stdout -----
        product: collie
        product_major: 0
        product_version: [PRODUCT_VERSION]
        protocol_version: 0.1
        ----- stderr -----
        ");
    });
}

/// Covers: story:11-204, command:engine.info
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn engine_info_json_prints_the_engine_info_and_nothing_else(#[case] mode: Mode) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["engine", "info", "--json"])
        .mode(mode)
        .json("EngineInfo")
        .unwrap();

    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    assert_eq!(checked.documents[0]["product"], json!("collie"));
}

/// Covers: story:11-197, command:run.start
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_start_prints_the_new_runs_id(#[case] mode: Mode) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .mode(mode)
        .run()
        .unwrap();

    assert_cli_snapshot!(printed(output), @r"
    exit: 0
    ----- stdout -----
    first
    ----- stderr -----
    ");
}

/// Covers: story:11-204, command:run.start
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_start_json_prints_the_run_started(#[case] mode: Mode) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first", "--json"])
        .mode(mode)
        .json("RunStarted")
        .unwrap();

    assert_cli_snapshot!(printed(checked.output), @r#"
    exit: 0
    ----- stdout -----
    {
      "lifecycle": "running",
      "needs": "collie",
      "run_id": "first",
      "started_as": "new"
    }
    ----- stderr -----
    "#);
}

/// Covers: story:11-197, story:11-204, command:run.finish
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_finish_reads_the_report_of_the_run_the_attached_engine_stopped(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--json"])
        .mode(mode)
        .json("RunReport")
        .unwrap();

    assert_cli_snapshot!(printed(checked.output), @r#"
    exit: 20
    ----- stdout -----
    {
      "completeness": "incomplete",
      "completeness_reasons": [
        {
          "code": "stopped",
          "message": "It stopped early: aborted by its Client."
        }
      ],
      "ended_at": "[INSTANT]",
      "lifecycle": "stopped",
      "run_id": "first",
      "started_at": "[INSTANT]",
      "stop_reason": {
        "by": "client",
        "kind": "abort"
      },
      "usage": {
        "browser_time": 0,
        "model": {
          "calls": 0,
          "tokens": 0
        },
        "proxy_bytes": 0,
        "requests": 0
      },
      "verdict": "verified"
    }
    ----- stderr -----
    "#);
}

/// Covers: command:run.abort
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_abort_prints_the_report_of_an_ended_run_one_field_per_line(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let output = cli(&env, COLLIE)
        .args(["run", "abort", "first"])
        .mode(mode)
        .run()
        .unwrap();

    assert_cli_snapshot!(printed(output), @r#"
    exit: 20
    ----- stdout -----
    completeness: incomplete
    completeness_reasons: [{"code":"stopped","message":"It stopped early: aborted by its Client."}]
    ended_at: [INSTANT]
    lifecycle: stopped
    run_id: first
    started_at: [INSTANT]
    stop_reason: {"by":"client","kind":"abort"}
    usage: {"browser_time":0,"model":{"calls":0,"tokens":0},"proxy_bytes":0,"requests":0}
    verdict: verified
    ----- stderr -----
    "#);
}

/// Covers: story:11-204, command:run.finish
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn a_failed_call_under_json_prints_the_error_with_its_code_group_and_operation(#[case] mode: Mode) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["run", "finish", "missing", "--json"])
        .mode(mode)
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
    assert_cli_snapshot!(
        "a_failed_call_under_json_prints_the_error_with_its_code_group_and_operation",
        printed(checked.output)
    );
}

/// Covers: command:run.finish
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn a_failed_call_prints_its_message_code_and_retryability_on_stderr(#[case] mode: Mode) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["run", "finish", "missing"])
        .mode(mode)
        .run()
        .unwrap();

    assert_eq!((output.exit, output.stdout.as_str()), (Some(3), ""));
    assert_cli_snapshot!(
        "a_failed_call_prints_its_message_code_and_retryability_on_stderr",
        printed(output)
    );
}

/// Covers: story:11-198, command:page.body
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_body_follow_prints_each_chunk_on_one_line_as_it_arrives(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");
    let body = store_body(&env, "first", b"hello");

    let output = cli(&env, COLLIE)
        .args([
            "page", "body", "--run-id", "first", "--body", &body, "--follow",
        ])
        .mode(mode)
        .run()
        .unwrap();

    assert_cli_snapshot!(printed(output), @r"
    exit: 0
    ----- stdout -----
    data=aGVsbG8= offset=0
    ----- stderr -----
    ");
}

/// Covers: command:run.usage
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_usage_answers_what_the_run_used(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", "usage", "first", "--json"])
        .mode(mode)
        .json("RunUsage")
        .unwrap();

    assert_eq!(
        (checked.output.exit, &checked.documents[0]),
        (
            Some(0),
            &json!({
                "browser_time": 0,
                "model": { "calls": 0, "tokens": 0 },
                "proxy_bytes": 0,
                "requests": 0
            })
        ),
        "{}",
        checked.output
    );
}

/// `run start` stops its Run when its engine exits with its Connection:
/// no engine works it, and its progress is the last value, which says how
/// it ended.
///
/// Covers: story:3-161, command:run.progress
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_progress_answers_the_last_progress_of_a_run_that_ended(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", "progress", "first", "--json"])
        .mode(mode)
        .json("RunProgress")
        .unwrap();

    let progress = &checked.documents[0];
    assert_eq!(
        (
            checked.output.exit,
            [
                &progress["writer"],
                &progress["lifecycle"],
                &progress["stop_reason"],
                &progress["last"]
            ]
        ),
        (
            Some(0),
            [
                &json!("none"),
                &json!("stopped"),
                &json!({ "kind": "abort", "by": "client" }),
                &json!(true)
            ]
        ),
        "{}",
        checked.output
    );
}

/// Covers: story:3-165, command:run.report
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_report_answers_the_report_of_a_run_that_ended_at_once(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", "report", "first", "--json"])
        .mode(mode)
        .json("RunReport")
        .unwrap();

    assert_eq!(
        (checked.output.exit, &checked.documents[0]["lifecycle"]),
        (Some(20), &json!("stopped")),
        "{}",
        checked.output
    );
}

/// `run list` answers the Run index's rows its flags name, in their
/// order, from the engine's own filter.
///
/// Covers: story:12-12, command:run.list
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_list_answers_the_rows_its_filter_names_in_its_order(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");
    start(&env, "second");

    let all = cli(&env, COLLIE)
        .args([
            "run",
            "list",
            "--sort",
            "run_id",
            "--order",
            "ascending",
            "--json",
        ])
        .mode(mode)
        .json("RunList")
        .unwrap();
    let matched = cli(&env, COLLIE)
        .args(["run", "list", "--text", "SEC", "--json"])
        .mode(mode)
        .json("RunList")
        .unwrap();

    let ids = |runs: &serde_json::Value| -> Vec<String> {
        runs["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["run_id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        (
            all.output.exit,
            ids(&all.documents[0]),
            ids(&matched.documents[0])
        ),
        (
            Some(0),
            vec!["first".to_owned(), "second".to_owned()],
            vec!["second".to_owned()]
        ),
        "{}",
        all.output
    );
}

/// Covers: command:run.checkpoint
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_checkpoint_writes_the_runs_checkpoint_and_answers_where(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", "checkpoint", "first", "--json"])
        .mode(mode)
        .json("Checkpoint")
        .unwrap();

    let checkpoint = &checked.documents[0];
    assert_eq!(
        (
            checked.output.exit,
            &checkpoint["run_id"],
            checkpoint["location"].as_str().map(PathBuf::from),
        ),
        (
            Some(0),
            &json!("first"),
            Some(run_folder(&env, "first").join("checkpoint.sqlite"))
        ),
        "{}",
        checked.output
    );
}

/// A command that ends a Run answers the report of a Run that already
/// ended as it is: `stopped` by its Client's abort, exiting `20`.
#[rstest]
/// Covers: command:run.pause
#[case::pause("pause")]
/// Covers: command:run.migrate
#[case::migrate("migrate")]
/// Covers: command:run.stop
#[case::stop("stop")]
fn a_command_that_ends_a_run_answers_an_ended_runs_report_as_it_is(
    #[case] command: &str,
    #[values(Mode::Attached, Mode::Daemon)] mode: Mode,
) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["run", command, "first", "--json"])
        .mode(mode)
        .json("RunReport")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["lifecycle"],
            &checked.documents[0]["stop_reason"],
        ),
        (
            Some(20),
            &json!("stopped"),
            &json!({ "by": "client", "kind": "abort" })
        ),
        "{}",
        checked.output
    );
}

/// The Key-value store of a Run no engine holds is read: a key never set
/// answers null.
///
/// Covers: command:key-value-store.get
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn key_value_store_get_answers_null_for_a_key_never_set(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args([
            "key-value-store",
            "get",
            "--run-id",
            "first",
            "--key",
            "progress",
            "--json",
        ])
        .mode(mode)
        .json("KeyValueStoreValue")
        .unwrap();

    assert_eq!(
        (checked.output.exit, &checked.documents[0]),
        (Some(0), &json!({ "value": null })),
        "{}",
        checked.output
    );
}

/// Bytes read from the store of a Run no engine holds: a key never set
/// answers null.
///
/// Covers: command:key-value-store.get-bytes
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn key_value_store_get_bytes_answers_null_for_a_key_never_set(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args([
            "key-value-store",
            "get-bytes",
            "--run-id",
            "first",
            "--key",
            "logo",
            "--json",
        ])
        .mode(mode)
        .json("KeyValueStoreBytesValue")
        .unwrap();

    assert_eq!(
        (checked.output.exit, &checked.documents[0]),
        (Some(0), &json!({ "bytes": null })),
        "{}",
        checked.output
    );
}

/// A store nothing was written to sends no key.
///
/// Covers: command:key-value-store.keys
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn key_value_store_keys_sends_none_from_a_store_never_written(#[case] mode: Mode) {
    let env = TestEnv::new();
    start(&env, "first");

    let output = cli(&env, COLLIE)
        .args([
            "key-value-store",
            "keys",
            "--run-id",
            "first",
            "--follow",
            "--json",
        ])
        .mode(mode)
        .run()
        .unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(0), ""),
        "{output}"
    );
}

/// The Key-value store of a Run no engine holds is never written: a write
/// fails with `run_not_found`, naming a start or a resume.
#[rstest]
/// Covers: command:key-value-store.set
#[case::set(&["set", "--value", "1"], "key_value_store.set")]
/// Covers: command:key-value-store.delete
#[case::delete(&["delete"], "key_value_store.delete")]
/// Covers: command:key-value-store.set-bytes
#[case::set_bytes(
    &["set-bytes", "--content", "aGk=", "--content-type", "text/plain"],
    "key_value_store.set_bytes"
)]
fn a_key_value_store_write_to_a_run_no_engine_holds_fails_with_run_not_found(
    #[case] command: &[&str],
    #[case] operation: &str,
    #[values(Mode::Attached, Mode::Daemon)] mode: Mode,
) {
    let env = TestEnv::new();
    start(&env, "first");

    let checked = cli(&env, COLLIE)
        .args(["key-value-store"])
        .args(command)
        .args(["--run-id", "first", "--key", "progress", "--json"])
        .mode(mode)
        .json("CommandFailed")
        .unwrap();

    let error = &checked.documents[0]["error"];
    assert_eq!(
        (
            checked.output.exit,
            &error["code"],
            &error["operation"],
            error["message"]
                .as_str()
                .is_some_and(|message| message.contains("start or resume it")),
        ),
        (Some(3), &json!("run_not_found"), &json!(operation), true),
        "{}",
        checked.output
    );
}
