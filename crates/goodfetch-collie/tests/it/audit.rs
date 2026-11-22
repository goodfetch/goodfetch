//! `audit export` and `audit verify` through Collie's executable (spec
//! #564's seam; ADRs 0016, 0066 and 0067): the user's Audit log, in the
//! Goodfetch home `GOODFETCH_HOME` names, written through the engine's
//! `record`, is verified by the CLI command and by `audit.verify` over the
//! protocol, which answer the same value: intact, or the first sequence that
//! does not verify. A log of a newer major is refused with the registry's
//! `format_version_unsupported`. `audit export` writes it as JSONL, one
//! entry with its hash per line, which `audit verify --file` verifies off
//! the machine; under `audit.retention` the engine prunes its beginning
//! behind an anchor, from which it still verifies.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::Arc;

use goodfetch_engine::audit::{AuditLog, Origin};
use goodfetch_engine::home::Home;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::audit::{Actor, SECRET_REFERENCE_USED};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_support::MESSAGE_DEADLINE;
use goodfetch_test_support::{Canaries, SecretKind, TestEnv, cli};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::engine_choice::{On, daemon_dispatched};
use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Writes `count` entries to the Audit log of `env`'s home through the
/// engine's own `record`.
fn written(env: &TestEnv, count: u64) -> AuditLog {
    written_at(env, count, "2026-01-05T09:00:00Z")
}

/// Writes `count` entries recorded at `time`, as [`written`] does. The log's
/// writer runs on a runtime of the call's own, so its store is closed when
/// the call returns; the log answered reads only.
fn written_at(env: &TestEnv, count: u64, time: &str) -> AuditLog {
    let log = AuditLog::of(
        &Home::new(env.home().to_path_buf()),
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    );
    let runtime = goodfetch_test_support::runtime();
    for index in 1..=count {
        runtime
            .block_on(log.record(
                &Origin {
                    actor: Actor::ClientSurface {
                        surface: Surface::Cli,
                    },
                    surface: None,
                    layer: None,
                    run_id: None,
                },
                &SECRET_REFERENCE_USED,
                json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
                time.parse().unwrap(),
            ))
            .unwrap();
    }
    log
}

/// What `audit verify --json` printed, and what `audit.verify` answered
/// an Attached engine's Client.
fn verified_both_ways(env: &TestEnv) -> (Value, Value) {
    let printed = cli(env, COLLIE)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();
    let mut engine = Attached::start(env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let answered = engine.ask(1, "audit.verify", &json!({}));
    let _ = engine.finish();
    (serde_json::from_str(&printed.stdout).unwrap(), answered)
}

/// Covers: story:4-69, story:4-70, command:audit.verify, operation:audit.verify
#[test]
fn audit_verify_finds_a_log_written_through_record_intact() {
    let env = TestEnv::new();
    written(&env, 3);

    let (printed, answered) = verified_both_ways(&env);

    assert_eq!(
        (printed, answered["result"].clone()),
        (
            json!({ "intact": true, "entries": 3 }),
            json!({ "intact": true, "entries": 3 })
        )
    );
}

/// `audit verify` and `audit export` read the log on the engine their
/// flags choose: an Attached engine in their process under `--attached`,
/// the Daemon of the major, which they start, under `--daemon`; either
/// prints the same (ticket #1183).
///
/// Covers: story:4-70, story:4-71, story:11-199
#[rstest]
/// Covers: command:audit.verify
#[case::verify_attached("verify", On::Attached, json!([{ "intact": true, "entries": 2 }]))]
/// Covers: command:audit.verify
#[case::verify_daemon("verify", On::Daemon, json!([{ "intact": true, "entries": 2 }]))]
/// Covers: command:audit.export
#[case::export_attached("export", On::Attached, json!([1, 2]))]
/// Covers: command:audit.export
#[case::export_daemon("export", On::Daemon, json!([1, 2]))]
fn audit_verify_and_export_run_on_the_engine_their_flags_choose(
    #[case] command: &str,
    #[case] on: On,
    #[case] printed: Value,
) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    written(&env, 2);

    let output = cli(&env, COLLIE)
        .args(["audit", command, "--json", on.flag()])
        .run()
        .unwrap();

    // An export prints one line per entry, whose sequences both engines
    // must agree on; a verification prints its one result.
    let lines = output
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let shown: Value = if command == "export" {
        lines.map(|entry| entry["sequence"].clone()).collect()
    } else {
        lines.collect()
    };
    assert_eq!(
        (
            output.exit,
            shown,
            daemon_dispatched(&env, &format!("audit.{command}")),
        ),
        (Some(0), printed, on == On::Daemon),
        "{output}"
    );
}

/// Covers: story:4-70, command:audit.verify, operation:audit.verify
#[test]
fn audit_verify_names_the_first_entry_an_edit_broke() {
    let env = TestEnv::new();
    let log = written(&env, 3);
    let connection = Connection::open(log.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER entries_never_updated;
             UPDATE entries SET entry = replace(entry, 'SECRET_2', 'SECRET_X') WHERE sequence = 2;",
        )
        .unwrap();

    let (printed, answered) = verified_both_ways(&env);

    assert_eq!(
        (printed, answered["result"].clone()),
        (
            json!({ "intact": false, "entries": 3, "first_broken": 2 }),
            json!({ "intact": false, "entries": 3, "first_broken": 2 })
        )
    );
}

/// Covers: format:audit-log, error:format_version_unsupported
#[test]
fn audit_verify_refuses_a_log_of_a_newer_major() {
    let env = TestEnv::new();
    let log = written(&env, 1);
    Connection::open(log.path())
        .unwrap()
        .execute("UPDATE format SET version = '1.0'", [])
        .unwrap();

    let refused = cli(&env, COLLIE)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap();
    assert_eq!(
        (
            &printed["error"]["code"],
            &printed["error"]["format"],
            &printed["error"]["found"]
        ),
        (
            &json!("format_version_unsupported"),
            &json!("audit-log"),
            &json!("1.0")
        )
    );
}

/// Each stored entry's sequence and hash, read from the store itself.
fn stored_hashes(log: &AuditLog) -> Vec<(i64, String)> {
    Connection::open(log.path())
        .unwrap()
        .prepare("SELECT sequence, hash FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// What `audit export` wrote to stdout.
fn exported(env: &TestEnv) -> String {
    cli(env, COLLIE)
        .args(["audit", "export"])
        .json_lines("ExportedAuditEntry")
        .unwrap()
        .printed
}

/// What `audit verify --file <file> --json` printed of `jsonl`, written to
/// a file of the test's folder.
fn verified_file(env: &TestEnv, jsonl: &str) -> Value {
    let file = env.folder().join("audit-export.jsonl");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the export it hands the command"
    )]
    std::fs::write(&file, jsonl).unwrap();
    let printed = cli(env, COLLIE)
        .args([
            "audit",
            "verify",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ])
        .run()
        .unwrap();
    serde_json::from_str(&printed.stdout).unwrap()
}

/// Covers: story:4-71, command:audit.export, operation:audit.export
#[test]
fn audit_export_writes_each_entry_with_its_hash_as_one_json_line_in_sequence() {
    let env = TestEnv::new();
    let log = written(&env, 3);

    let printed = exported(&env);

    let lines: Vec<(u64, String, String)> = printed
        .lines()
        .map(|line| {
            let entry: Value = serde_json::from_str(line).unwrap();
            (
                entry["sequence"].as_u64().unwrap(),
                entry["details"]["reference"].as_str().unwrap().to_owned(),
                entry["hash"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let stored = stored_hashes(&log);
    assert_eq!(
        lines,
        vec![
            (1, "env:SECRET_1".to_owned(), stored[0].1.clone()),
            (2, "env:SECRET_2".to_owned(), stored[1].1.clone()),
            (3, "env:SECRET_3".to_owned(), stored[2].1.clone()),
        ]
    );
}

/// Covers: story:4-71, operation:audit.export
#[test]
fn audit_export_answers_over_the_protocol_the_entries_the_cli_writes() {
    let env = TestEnv::new();
    written(&env, 2);
    let printed: Vec<Value> = exported(&env)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let answered = engine.ask(1, "audit.export", &json!({}));

    let _ = engine.finish();
    assert_eq!(answered["result"]["entries"], Value::Array(printed));
}

/// `audit verify` exits `22`, `audit_log_not_intact`, for a log or an
/// export that does not verify, as `sha256sum -c` exits non-zero on a
/// mismatch, and `0` for an intact one, so a gate that reads only the exit
/// code catches an edit (ADR 0069).
///
/// Covers: story:4-70, story:4-71, command:audit.verify, exit:audit_log_not_intact
#[test]
fn audit_verify_exits_22_for_a_log_or_export_that_does_not_verify() {
    let env = TestEnv::new();
    let log = written(&env, 3);
    let jsonl = exported(&env);
    let file = env.folder().join("edited-export.jsonl");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the export it hands the command"
    )]
    std::fs::write(&file, jsonl.replace("SECRET_2", "SECRET_X")).unwrap();
    let intact = cli(&env, COLLIE).args(["audit", "verify"]).run().unwrap();
    Connection::open(log.path())
        .unwrap()
        .execute_batch(
            "DROP TRIGGER entries_never_updated;
             UPDATE entries SET entry = replace(entry, 'SECRET_2', 'SECRET_X') WHERE sequence = 2;",
        )
        .unwrap();

    let edited = cli(&env, COLLIE)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();
    let edited_export = cli(&env, COLLIE)
        .args(["audit", "verify", "--file", file.to_str().unwrap()])
        .run()
        .unwrap();

    assert_eq!(
        (
            intact.exit,
            edited.exit,
            serde_json::from_str::<Value>(&edited.stdout).ok(),
            edited_export.exit,
            edited_export.stdout.contains("first_broken: 2"),
        ),
        (
            Some(0),
            Some(22),
            Some(json!({ "intact": false, "entries": 3, "first_broken": 2 })),
            Some(22),
            true,
        ),
        "{intact}\n{edited}\n{edited_export}"
    );
}

/// The positive control and the tampered copy: the export as written
/// verifies intact off the machine, and the same export with one entry
/// edited is reported at it.
/// Covers: story:4-71, command:audit.export, command:audit.verify
#[test]
fn an_exported_log_verifies_intact_and_an_edit_to_it_shows() {
    let env = TestEnv::new();
    written(&env, 3);
    let jsonl = exported(&env);

    let (intact, edited) = (
        verified_file(&env, &jsonl),
        verified_file(&env, &jsonl.replace("SECRET_2", "SECRET_X")),
    );

    assert_eq!(
        (intact, edited),
        (
            json!({ "intact": true, "entries": 3 }),
            json!({ "intact": false, "entries": 3, "first_broken": 2 })
        )
    );
}

/// `config explain audit` shows each of the Audit log's keys with the
/// layer that set it: `audit.retention` from the user configuration, and
/// the defaults of `audit.otlp`, off, and `audit.refusals_per_minute`, 10
/// (#2324).
/// Covers: story:4-72, command:config.explain, setting:audit.retention, setting:audit.otlp
#[test]
fn config_explain_audit_shows_its_retention_and_otlp_keys_with_their_layer() {
    let env = TestEnv::new();
    env.user_configuration("audit.retention = 15552000000\n");

    let printed = cli(&env, COLLIE)
        .args(["config", "explain", "audit", "--json"])
        .run()
        .unwrap();

    let explained: Value = serde_json::from_str(&printed.stdout).unwrap();
    let keys: Vec<Value> = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| json!([setting["key"], setting["value"], setting["layer"]]))
        .collect();
    assert_eq!(
        keys,
        [
            json!(["audit.otlp", false, "default"]),
            json!(["audit.refusals_per_minute", 10, "default"]),
            json!(["audit.retention", 15_552_000_000_u64, "user_configuration"]),
        ]
    );
}

/// The variable the engine reads the Run's key-encryption key from, by
/// `run_state.key_reference`, whose use the engine records.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

/// Starts and finishes a Run on an Attached engine, which records the use
/// of its key reference in the Audit log; answers the Run's lifecycle.
pub(crate) fn recorded_by_a_run(env: &TestEnv) -> Value {
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let mut engine = Attached::start_with(env, COLLIE, MESSAGE_DEADLINE, |command| {
        command.env(KEY_VARIABLE, &key);
    });
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.ask(1, "run.start", &json!({ "run_id": "audited" }));
    let finished = engine.ask(2, "run.finish", &json!({ "run_id": "audited" }));
    let _ = engine.finish();
    finished["result"]["lifecycle"].clone()
}

/// The user configuration that names the Run's key by reference, then
/// `more`.
pub(crate) fn key_by_reference(more: &str) -> String {
    format!("run_state.key_reference = \"env:{KEY_VARIABLE}\"\n{more}")
}

/// Under `audit.retention`, the engine prunes as it starts the entries
/// older than it behind an anchor carrying the last pruned hash, and
/// `audit verify` reports the log intact from it; the export starts with
/// the anchor, then the pruning record and the Run's entry, in the order
/// the start's pass and the Run's append reached the log.
/// Covers: story:4-72, setting:audit.retention, command:audit.verify, command:audit.export, operation:audit.verify, operation:audit.export
#[test]
fn under_audit_retention_an_engine_prunes_the_old_entries_and_the_log_verifies_from_its_anchor() {
    let env = TestEnv::new();
    env.user_configuration(&key_by_reference("audit.retention = 86400000\n"));
    let log = written_at(&env, 2, "2020-01-05T09:00:00Z");
    let last_pruned = stored_hashes(&log)[1].1.clone();

    let lifecycle = recorded_by_a_run(&env);

    let entries: Vec<Value> = exported(&env)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let first = entries.first().map(|entry| entry["sequence"].clone());
    let mut kinds: Vec<Value> = entries
        .iter()
        .map(|entry| json!([entry["kind"], entry["hash"] == json!(last_pruned)]))
        .collect();
    kinds[1..].sort_by_key(ToString::to_string);
    let (printed, _) = verified_both_ways(&env);
    assert_eq!(
        (lifecycle, first, kinds, printed),
        (
            json!("finished"),
            Some(json!(2)),
            vec![
                json!(["audit_log_anchor", true]),
                json!(["audit_log_pruned", false]),
                json!(["secret_reference_used", false]),
            ],
            json!({ "intact": true, "entries": 3 })
        )
    );
}

/// With `audit.retention` unset, the default, the same old entries stay.
/// Covers: story:4-72, setting:audit.retention, command:audit.export, operation:audit.export
#[test]
fn with_audit_retention_unset_an_engine_prunes_nothing() {
    let env = TestEnv::new();
    env.user_configuration(&key_by_reference(""));
    written_at(&env, 2, "2020-01-05T09:00:00Z");

    let lifecycle = recorded_by_a_run(&env);

    let kinds: Vec<Value> = exported(&env)
        .lines()
        .map(|line| {
            let entry: Value = serde_json::from_str(line).unwrap();
            json!([entry["sequence"], entry["kind"]])
        })
        .collect();
    assert_eq!(
        (lifecycle, kinds),
        (
            json!("finished"),
            vec![
                json!([1, "secret_reference_used"]),
                json!([2, "secret_reference_used"]),
                json!([3, "secret_reference_used"]),
            ]
        )
    );
}

/// Each OpenTelemetry event of the exports `requests`: its event name, its
/// body, and its attributes `goodfetch.audit.sequence`, `.kind` and `.hash`.
fn events_of(requests: impl Iterator<Item = ExportLogsServiceRequest>) -> Vec<Value> {
    let text = |value: &Option<AnyValue>| match value {
        Some(AnyValue::StringValue(text)) => json!(text),
        Some(AnyValue::IntValue(number)) => json!(number),
        _ => Value::Null,
    };
    requests
        .flat_map(|request| request.resource_logs)
        .flat_map(|resource| resource.scope_logs)
        .flat_map(|scope| scope.log_records)
        .map(|record| {
            let attribute = |key: &str| {
                record
                    .attributes
                    .iter()
                    .find(|attribute| attribute.key == key)
                    .map_or(Value::Null, |attribute| {
                        text(
                            &attribute
                                .value
                                .as_ref()
                                .and_then(|value| value.value.clone()),
                        )
                    })
            };
            let body = text(&record.body.as_ref().and_then(|body| body.value.clone()));
            json!([
                record.event_name,
                body.as_str()
                    .map_or(Value::Null, |body| serde_json::from_str(body).unwrap()),
                attribute("goodfetch.audit.sequence"),
                attribute("goodfetch.audit.kind"),
                attribute("goodfetch.audit.hash"),
            ])
        })
        .collect()
}

/// With `otlp.logs_endpoint` naming a collector, each new entry the engine
/// appends reaches it as an OpenTelemetry event, the exported line as its
/// body, only while `audit.otlp` is on; the entry is in the log either way.
/// Covers: story:4-71, setting:audit.otlp, setting:otlp.logs_endpoint, command:audit.export, operation:audit.export
#[rstest]
#[case::with_audit_otlp_on(true)]
#[case::with_audit_otlp_off(false)]
fn each_new_entry_reaches_the_otlp_collector_as_an_event_only_while_audit_otlp_is_on(
    #[case] on: bool,
) {
    let env = TestEnv::new();
    let (endpoint, exports, _) = crate::log::collector_of::<ExportLogsServiceRequest>("/v1/logs");
    env.user_configuration(&key_by_reference(&format!(
        "otlp.logs_endpoint = \"{endpoint}\"\naudit.otlp = {on}\n"
    )));

    let lifecycle = recorded_by_a_run(&env);

    let events = events_of(exports.try_iter());
    let line: Value = serde_json::from_str(exported(&env).trim()).unwrap();
    let expected = if on {
        vec![json!([
            "goodfetch.audit",
            line,
            1,
            "secret_reference_used",
            line["hash"]
        ])]
    } else {
        vec![]
    };
    assert_eq!((lifecycle, events), (json!("finished"), expected));
}
