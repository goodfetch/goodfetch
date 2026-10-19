//! `goodfetch-test-env`'s command line and its JSON description (ADR 0062),
//! the contract the TypeScript and Python packages read.

use std::io::{BufRead as _, BufReader};
use std::path::Path;

use goodfetch_test_support::{Process, TOOLS_VARIABLE, TestEnv, command};
use serde_json::Value;

use crate::{TEST_ENV, manifest};

/// The committed schema of the description, which `xtask codegen` writes.
const SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/test-env-description.schema.json"
);

/// Starts `goodfetch-test-env` with these arguments.
fn start(env: &TestEnv, arguments: &[&str]) -> Process {
    let mut command = env.process_command(TEST_ENV);
    command.args(arguments).env(TOOLS_VARIABLE, manifest(env));
    env.spawn(&mut command).unwrap()
}

/// The first line it writes on stdout.
fn first_line(process: &Process) -> String {
    let stdout = process.child().stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    line
}

#[test]
fn description_is_one_json_line_that_validates_against_the_committed_schema() {
    let env = TestEnv::new();
    let process = start(&env, &[]);

    let line = first_line(&process);

    process.close_stdin_and_wait().unwrap();
    let description: Value = serde_json::from_str(line.trim_end_matches('\n')).unwrap();
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    let errors: Vec<String> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&description)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(errors, Vec::<String>::new(), "{line}");
    assert!(line.ends_with('\n') && !line.trim_end().contains('\n'));
}

#[test]
fn a_description_the_schema_refuses_fails_validation() {
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    let description = serde_json::json!({ "folder": "/tmp/x", "seed": -1 });

    let valid = jsonschema::validator_for(&schema)
        .unwrap()
        .is_valid(&description);

    assert!(!valid);
}

#[test]
fn closing_stdin_removes_the_folder_and_leaves_no_process() {
    let env = TestEnv::new();
    let process = start(&env, &[]);
    let description: Value = serde_json::from_str(&first_line(&process)).unwrap();
    let folder = description["folder"].as_str().unwrap().to_owned();
    assert!(Path::new(&folder).is_dir(), "{folder}");

    let status = process.close_stdin_and_wait().unwrap();

    assert!(status.success());
    assert!(!Path::new(&folder).exists(), "{folder}");
}

#[test]
fn locale_and_time_zone_set_lang_and_tz() {
    let env = TestEnv::new();
    let process = start(
        &env,
        &["--locale", "ja-JP", "--time-zone", "Pacific/Chatham"],
    );

    let description: Value = serde_json::from_str(&first_line(&process)).unwrap();

    process.close_stdin_and_wait().unwrap();
    assert_eq!(
        (&description["env"]["LANG"], &description["env"]["TZ"]),
        (&Value::from("ja_JP.UTF-8"), &Value::from("Pacific/Chatham"))
    );
}

#[test]
fn test_name_names_the_folder() {
    let env = TestEnv::new();
    let process = start(&env, &["--test-name", "sdk::resumes_a_run"]);

    let description: Value = serde_json::from_str(&first_line(&process)).unwrap();

    process.close_stdin_and_wait().unwrap();
    let folder = Path::new(description["folder"].as_str().unwrap());
    let name = folder.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("sdk-resumes_a_run é "), "{name}");
}

#[test]
fn a_failed_start_exits_non_zero_with_a_message_on_stderr() {
    let env = TestEnv::new();

    let output = command(&env, TEST_ENV)
        .env(TOOLS_VARIABLE, manifest(&env))
        .env("GOODFETCH_TEST_SEED", "seventeen")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert_ne!(output.stderr, b"");
}

#[test]
fn print_schemas_starts_nothing() {
    let env = TestEnv::new();

    let output = command(&env, TEST_ENV)
        .arg("--print-schemas")
        .output()
        .unwrap();

    assert!(output.status.success());
    let schemas: serde_json::Map<String, Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        schemas.keys().collect::<Vec<_>>(),
        [
            "endpoint-table.schema.json",
            "test-env-description.schema.json",
            "test-tool-manifest.schema.json"
        ]
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("goodfetch-test-seed="), "{stderr}");
}
