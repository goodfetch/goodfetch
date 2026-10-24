//! J-027, "Switch a security control off for one run, then export the
//! Audit log and verify its chain", on the `cli` surface (ADRs 0016, 0063
//! and 0075): a person names the runs' key-encryption key by a Secret
//! reference in user configuration, starts one run with
//! `security.audit_chain` off and another with every control on through
//! Collie's shipped executable, reads the first run's report, then exports
//! the Audit log and verifies the export and the log itself.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the exported Audit log to a file"
)]

use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{Canaries, JourneyRun, SecretKind};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The variable the engine reads the runs' key-encryption key from.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

/// The run started with the Audit log's hash chain off.
const UNPROTECTED: &str = "unprotected";

/// The run started with every control on.
const PROTECTED: &str = "protected";

/// What a command printed: its stdout and its stderr.
fn printed(output: &std::process::Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The exported entries of `run_id`, each as its kind and whether it is
/// chained (`chained` is left out when it is).
fn entries_of(exported: &[Value], run_id: &str) -> Vec<Value> {
    exported
        .iter()
        .filter(|entry| entry["run_id"] == json!(run_id))
        .map(|entry| {
            json!([
                entry["kind"],
                entry.get("chained").cloned().unwrap_or(json!(true))
            ])
        })
        .collect()
}

/// Covers: story:4-69, story:4-70, story:4-71, story:4-74, row:compliance-security-privacy
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-027", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-027's cli variant starts");
    let mut canaries = Canaries::new(run.env().seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let mut started_stderr = String::new();
    let mut report = Value::Null;
    let mut exported: Vec<Value> = Vec::new();
    let mut verified_export = Value::Null;
    let mut verified_log = Value::Null;

    run.step(
        "Name the runs' key-encryption key by a Secret reference in the user configuration, so that each run records its use in the Audit log",
        |run| {
            run.env()
                .user_configuration(&format!("run_state.key_reference = \"env:{KEY_VARIABLE}\"\n"));
        },
    );
    run.step(
        "Start a run with the Audit log's hash chain switched off for that run only",
        |run| {
            let output = run
                .command()
                .env(KEY_VARIABLE, &key)
                .args(["run", "start", "--run-id", UNPROTECTED, "--attached"])
                .args(["--set", "security.audit_chain=false"])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            started_stderr = printed(&output).1;
        },
    );
    run.step("Start a second run with every security control on", |run| {
        let output = run
            .command()
            .env(KEY_VARIABLE, &key)
            .args(["run", "start", "--run-id", PROTECTED, "--attached"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    });
    run.step("Read the first run's report", |run| {
        let output = run
            .command()
            .env(KEY_VARIABLE, &key)
            .args(["run", "finish", UNPROTECTED, "--attached", "--json"])
            .output()
            .unwrap();
        report = serde_json::from_str(&printed(&output).0).unwrap();
    });
    run.step("Export the Audit log", |run| {
        let output = run.command().args(["audit", "export"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let jsonl = printed(&output).0;
        std::fs::write(run.env().folder().join("audit.jsonl"), &jsonl).unwrap();
        exported = jsonl
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    });
    run.step("Verify the exported log, and the Audit log itself", |run| {
        let file = run.env().folder().join("audit.jsonl");
        let output = run
            .command()
            .args(["audit", "verify", "--json", "--file"])
            .arg(&file)
            .output()
            .unwrap();
        verified_export = serde_json::from_str(&printed(&output).0).unwrap();
        let output = run
            .command()
            .args(["audit", "verify", "--json"])
            .output()
            .unwrap();
        verified_log = serde_json::from_str(&printed(&output).0).unwrap();
    });

    run.check(
        "The first run's report lists the Audit log's hash chain in `weakened`, and the engine logged one warning naming it",
        |_| {
            let warnings = started_stderr
                .lines()
                .filter(|line| line.contains("WARN") && line.contains("security.audit_chain"))
                .count();
            assert_eq!(
                (&report["weakened"], warnings),
                (
                    &json!([{
                        "key": "security.audit_chain",
                        "value": false,
                        "layer": "run",
                        "origin": "settings",
                    }]),
                    1
                ),
                "{started_stderr}"
            );
        },
    );
    run.check(
        "The Audit log holds one chained entry recording the switch for the first run, and the first run's other entries are marked unchained",
        |_| {
            let switches: Vec<Value> = exported
                .iter()
                .filter(|entry| entry["kind"] == json!("security_switch_turned_off"))
                .map(|entry| json!([entry["run_id"], entry["details"]["setting"], entry["layer"]]))
                .collect();
            assert_eq!(
                (switches, entries_of(&exported, UNPROTECTED)),
                (
                    vec![json!([UNPROTECTED, "security.audit_chain", "run"])],
                    vec![
                        json!(["secret_reference_used", false]),
                        json!(["security_switch_turned_off", true]),
                    ]
                )
            );
        },
    );
    run.check("The second run's entries are chained", |_| {
        assert_eq!(
            entries_of(&exported, PROTECTED),
            vec![json!(["secret_reference_used", true])]
        );
    });
    run.check(
        "`audit verify` reports the exported log and the Audit log itself intact",
        |_| {
            assert_eq!(
                (&verified_export, &verified_log),
                (
                    &json!({ "intact": true, "entries": 3 }),
                    &json!({ "intact": true, "entries": 3 })
                )
            );
        },
    );
    run.finish()
        .expect("J-027's document is written")
        .assert_proved();
}
