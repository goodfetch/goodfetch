//! The Audit log's commands through Foxhound's executable, which carries
//! Collie's `audit export` and `audit verify` unchanged (ADR 0095): a Run
//! on Foxhound's Attached engine records the use of its key reference, and
//! the export, the export's verification and the log's give what Collie's
//! executable gives for the same log (`goodfetch-collie`'s
//! `tests/it/audit.rs`).

use std::io::Write as _;
use std::process::Stdio;

use goodfetch_test_support::{Canaries, SecretKind, StrictStdout, TestEnv, cli, record_command};
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The variable the engine reads the Run's key-encryption key from.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

fn line(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// Starts a Run on Foxhound's Attached engine, which records its key
/// reference's use in the Audit log as it starts; answers the replies.
fn recorded_by_a_run(env: &TestEnv) -> Vec<Value> {
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let mut command = env.process_command(FOXHOUND);
    command
        .arg("attached-engine")
        .env(KEY_VARIABLE, key)
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let mut stdin = process.child().stdin.take().unwrap();
    let mut stdout = StrictStdout::of(&process).unwrap();
    for message in [
        json!({
            "jsonrpc": "2.0", "id": 0, "method": "hello",
            "params": {
                "protocol_version": "0.1",
                "surface": "sdk_python",
                "needs": "collie",
                "product_version": "0.0.0",
            },
        }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "run.start", "params": { "run_id": "audited" } }),
    ] {
        stdin.write_all(&line(&message)).unwrap();
    }
    drop(stdin);
    let replies: Vec<Value> = std::iter::from_fn(|| stdout.try_next().unwrap()).collect();
    let status = process.child().wait().unwrap();
    record_command(env, &["attached-engine"], status.code()).unwrap();
    replies
}

/// Covers: story:4-71, command:audit.export, command:audit.verify, operation:audit.export, operation:audit.verify
#[test]
fn foxhounds_audit_export_and_verify_give_the_results_collies_give() {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "run_state.key_reference = \"env:{KEY_VARIABLE}\"\n"
    ));
    let replies = recorded_by_a_run(&env);

    let exported = cli(&env, FOXHOUND)
        .args(["audit", "export"])
        .json_lines("ExportedAuditEntry")
        .unwrap();
    let file = env.folder().join("audit-export.jsonl");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the export it hands the command"
    )]
    std::fs::write(&file, &exported.printed).unwrap();
    let verified_file = cli(&env, FOXHOUND)
        .args([
            "audit",
            "verify",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ])
        .run()
        .unwrap();
    let verified_log = cli(&env, FOXHOUND)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();

    let entries: Vec<Value> = exported
        .documents
        .iter()
        .map(|entry| {
            json!([
                entry["sequence"],
                entry["kind"],
                entry["details"]["reference"]
            ])
        })
        .collect();
    let printed = |stdout: &str| serde_json::from_str::<Value>(stdout).unwrap();
    let finished = replies
        .iter()
        .find(|reply| reply["id"] == json!(1))
        .map(|reply| &reply["result"]["lifecycle"]);
    assert_eq!(
        (
            finished,
            entries,
            printed(&verified_file.stdout),
            printed(&verified_log.stdout),
        ),
        (
            Some(&json!("running")),
            vec![json!([
                1,
                "secret_reference_used",
                format!("env:{KEY_VARIABLE}")
            ])],
            json!({ "intact": true, "entries": 1 }),
            json!({ "intact": true, "entries": 1 }),
        )
    );
}
