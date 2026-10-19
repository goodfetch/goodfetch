//! `goodfetch-test-env`'s command line and its JSON description (ADR 0062),
//! the contract the TypeScript and Python packages read.

use std::io::{BufRead as _, BufReader};
use std::path::Path;

use goodfetch_test_support::{
    CRASH_DUMP_VARIABLE, Process, TOOLS_VARIABLE, TestEnv, Tool, command, tools,
};
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
fn a_dump_in_the_crash_dump_folder_fails_the_teardown_closing_stdin_runs() {
    let env = TestEnv::new();
    let process = start(&env, &[]);
    let description: Value = serde_json::from_str(&first_line(&process)).unwrap();
    let crash_dumps = Path::new(description["env"][CRASH_DUMP_VARIABLE].as_str().unwrap());
    std::fs::write(crash_dumps.join("renderer.dmp"), "dump").unwrap();

    let status = process.close_stdin_and_wait().unwrap();

    let folder = Path::new(description["folder"].as_str().unwrap());
    let kept = folder.is_dir();
    if kept {
        std::fs::remove_dir_all(folder).unwrap();
    }
    assert_eq!((status.success(), kept), (false, true));
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
            "locales.schema.json",
            "snapshot-filters.schema.json",
            "ssrf-cases.schema.json",
            "test-env-description.schema.json",
            "test-tool-manifest.schema.json",
            "unicode-record.schema.json"
        ]
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("goodfetch-test-seed="), "{stderr}");
}

/// Writes a test tool manifest listing `goodfetch-test-env` and the commit's
/// Proving ground, from the manifest `just test` writes, into the test's
/// folder, and returns its path.
fn manifest_with_proving_ground(env: &TestEnv) -> std::path::PathBuf {
    let proving_ground = tools()
        .unwrap()
        .path(Tool::ProvingGround)
        .unwrap()
        .to_owned();
    let path = env.folder().join("tools.json");
    let manifest = serde_json::json!({ "test_env": TEST_ENV, "proving_ground": proving_ground });
    std::fs::write(&path, manifest.to_string()).unwrap();
    path
}

/// Starts `goodfetch-test-env --proving-ground <seed>` with that manifest.
fn start_with_proving_ground(env: &TestEnv, seed: &str) -> Process {
    let mut command = env.process_command(TEST_ENV);
    command
        .args(["--proving-ground", seed])
        .env(TOOLS_VARIABLE, manifest_with_proving_ground(env));
    env.spawn(&mut command).unwrap()
}

/// Reads a JSON answer to `GET <path>` on `address`.
#[expect(
    clippy::disallowed_types,
    reason = "the test reaches the described Proving ground over a plain socket"
)]
fn get_json(address: &str, path: &str) -> Value {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[test]
fn proving_ground_adds_a_started_proving_ground_that_validates_against_the_committed_schema() {
    let env = TestEnv::new();
    let process = start_with_proving_ground(&env, "7");

    let line = first_line(&process);

    let description: Value = serde_json::from_str(line.trim_end_matches('\n')).unwrap();
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    let errors: Vec<String> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&description)
        .map(|error| error.to_string())
        .collect();
    let tasks = description["proving_ground"]["address"]
        .as_str()
        .map(|address| get_json(address, "/tasks"));
    process.close_stdin_and_wait().unwrap();
    assert_eq!(errors, Vec::<String>::new(), "{line}");
    assert_eq!(description["proving_ground"]["seed"], 7, "{line}");
    assert_eq!(tasks.unwrap()["proving_ground"]["seed"], 7);
}

#[test]
fn closing_stdin_stops_the_proving_ground() {
    let env = TestEnv::new();
    let process = start_with_proving_ground(&env, "7");
    let line = first_line(&process);
    let description: Value = serde_json::from_str(&line).unwrap();
    let address = description["proving_ground"]["address"]
        .as_str()
        .unwrap_or_else(|| panic!("no Proving ground: {line}"))
        .to_owned();

    let status = process.close_stdin_and_wait().unwrap();

    #[expect(
        clippy::disallowed_types,
        reason = "the test checks that nothing answers on the address any more"
    )]
    let connected = std::net::TcpStream::connect(&address).map_err(|error| error.kind());
    assert!(status.success());
    assert_eq!(connected.err(), Some(std::io::ErrorKind::ConnectionRefused));
}

#[test]
fn a_description_without_proving_ground_has_no_such_field() {
    let env = TestEnv::new();
    let process = start(&env, &[]);

    let description: Value = serde_json::from_str(&first_line(&process)).unwrap();

    process.close_stdin_and_wait().unwrap();
    assert_eq!(description.get("proving_ground"), None);
}

#[test]
fn a_proving_ground_without_its_resolve_mapping_fails_validation() {
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    let description = serde_json::json!({
        "folder": "/tmp/x é 1", "home": "/tmp/x é 1/home", "env": {}, "seed": 1,
        "start_time": "2026-01-05T09:00:00Z", "tools": {},
        "proving_ground": {
            "address": "127.0.0.1:4000", "version": "0.0.0+0", "seed": 7, "ca": "/tmp/ca.pem",
            "fixtures": {
                "proxies": {}, "proxy_credentials": { "username": "u", "password": "p" },
                "dns": "127.0.0.1:4001", "target": "127.0.0.1:4002",
                "connections": "http://127.0.0.1:4000/observations"
            }
        }
    });
    let mut complete = description.clone();
    complete["proving_ground"]["resolve"] = serde_json::json!({ "catalog.test": "127.0.0.1" });
    let validator = jsonschema::validator_for(&schema).unwrap();

    let valid = (
        validator.is_valid(&description),
        validator.is_valid(&complete),
    );

    assert_eq!(valid, (false, true));
}

#[test]
fn a_proving_ground_that_cannot_start_fails_the_start() {
    let env = TestEnv::new();

    let output = command(&env, TEST_ENV)
        .args(["--proving-ground", "7"])
        .env(TOOLS_VARIABLE, manifest(&env))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert_ne!(output.stderr, b"");
}
