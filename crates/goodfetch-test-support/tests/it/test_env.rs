//! `goodfetch-test-env`'s command line and its JSON description (ADR 0062),
//! the contract the TypeScript and Python packages read.

use std::io::{BufRead as _, BufReader};
use std::path::Path;

use goodfetch_test_support::{
    CRASH_DUMP_VARIABLE, Process, TOOLS_VARIABLE, TestEnv, Tool, command, schemas, tools,
};
use serde_json::Value;

use crate::{TEST_ENV, get, manifest, manifest_with, schema_errors};

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
    let errors = schema_errors(&description);
    assert_eq!(errors, Vec::<String>::new(), "{line}");
    assert!(line.ends_with('\n') && !line.trim_end().contains('\n'));
}

#[test]
fn a_description_the_schema_refuses_fails_validation() {
    let description = serde_json::json!({ "folder": "/tmp/x", "seed": -1 });

    let errors = schema_errors(&description);

    assert_ne!(errors, Vec::<String>::new());
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

/// A package whose test failed writes `failed` before it closes stdin: the
/// folder is kept for the failure's evidence, as a Rust test's is, and its
/// path is printed.
#[test]
fn a_failed_tests_folder_is_kept() {
    let env = TestEnv::new();
    let process = start(&env, &[]);
    let description: Value = serde_json::from_str(&first_line(&process)).unwrap();
    let folder = description["folder"].as_str().unwrap().to_owned();

    {
        use std::io::Write as _;
        let mut stdin = process.child().stdin.take().unwrap();
        stdin.write_all(b"failed\n").unwrap();
    }
    let status = process.close_stdin_and_wait().unwrap();

    assert!(status.success());
    assert!(Path::new(&folder).is_dir(), "{folder}");
    std::fs::remove_dir_all(&folder).unwrap();
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
    // Every schema the crate defines, by its path: `xtask codegen` commits
    // exactly what it prints, and its check names a schema left uncommitted.
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed, serde_json::to_value(schemas()).unwrap());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("goodfetch-test-seed="), "{stderr}");
}

/// Starts `goodfetch-test-env --proving-ground <seed>` with that manifest.
fn start_with_proving_ground(env: &TestEnv, seed: &str) -> Process {
    let mut command = env.process_command(TEST_ENV);
    command.args(["--proving-ground", seed]).env(
        TOOLS_VARIABLE,
        manifest_with(
            env,
            &[(
                "proving_ground",
                tools().unwrap().path(Tool::ProvingGround).unwrap(),
            )],
        ),
    );
    env.spawn(&mut command).unwrap()
}

#[test]
fn proving_ground_adds_a_started_proving_ground_that_validates_against_the_committed_schema() {
    let env = TestEnv::new();
    let process = start_with_proving_ground(&env, "7");

    let line = first_line(&process);

    let description: Value = serde_json::from_str(line.trim_end_matches('\n')).unwrap();
    let errors = schema_errors(&description);
    let tasks = description["proving_ground"]["address"]
        .as_str()
        .map(|address| get(address, address, "/tasks").json());
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
    let description = serde_json::json!({
        "folder": "/tmp/x é 1", "home": "/tmp/x é 1/home", "env": {}, "seed": 1,
        "start_time": "2026-01-05T09:00:00Z", "tools": {},
        "proving_ground": {
            "address": "127.0.0.1:4000", "version": "0.0.0+0", "seed": 7, "ca": "/tmp/ca.pem",
            "fixtures": {
                "proxies": {}, "proxy_credentials": { "username": "u", "password": "p" },
                "dns": "127.0.0.1:4001", "target": "127.0.0.1:4002", "udp": "127.0.0.1:4003",
                "connections": "http://127.0.0.1:4000/observations"
            }
        }
    });
    let mut complete = description.clone();
    complete["proving_ground"]["resolve"] = serde_json::json!({ "catalog.test": "127.0.0.1" });
    let valid = (
        schema_errors(&description).is_empty(),
        schema_errors(&complete).is_empty(),
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
