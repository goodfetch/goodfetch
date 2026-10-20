//! The hostile-input case tables (ADR 0067): each committed file against
//! its committed JSON Schema, the contract another language's runner reads
//! it by, and every Proving ground page a case names against the task list
//! of the commit's Proving ground, which serves it end to end.
#![expect(
    clippy::disallowed_types,
    reason = "this test reaches the Proving ground over a plain socket, as any client of it does"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use goodfetch_test_support::{HostileTables, ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::Value;

/// The schema's errors for `table`, written as TOML.
fn schema_errors(schema: &str, table: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(schema).unwrap();
    let table: Value = toml::from_str(table).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&table)
        .map(|error| error.to_string())
        .collect()
}

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../test-support/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[rstest]
#[case::header_values("header_values")]
#[case::secret_redirects("secret_redirects")]
#[case::paths("paths")]
#[case::schema_refs("schema_refs")]
#[case::limits("limits")]
fn a_committed_table_validates_against_its_committed_schema(#[case] family: &str) {
    let schema = fixture(&format!(
        "schemas/hostile-{}.schema.json",
        family.replace('_', "-")
    ));
    let table = fixture(&format!("hostile/{family}.toml"));

    let errors = schema_errors(&schema, &table);

    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn a_case_with_no_expected_error_fails_the_schema() {
    let schema = fixture("schemas/hostile-header-values.schema.json");
    let table = r#"
        [[cases]]
        name = "recipe_header_with_crlf"
        input = { source = "recipe", name = "X-Note", value = "a\r\nX-Injected: yes" }
        served_by = "program_input"
    "#;

    let errors = schema_errors(&schema, table);

    assert_ne!(errors, Vec::<String>::new());
}

/// The task IDs of the Proving ground's own task list.
fn task_ids(proving_ground: &ProvingGround) -> Vec<String> {
    let address = proving_ground.address().to_string();
    let mut stream = TcpStream::connect(&address).unwrap();
    write!(
        stream,
        "GET /tasks HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let tasks: Value = serde_json::from_str(body).unwrap();
    tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn every_page_a_committed_case_names_is_a_proving_ground_task() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, env.seed()).unwrap();
    let tasks = task_ids(&proving_ground);
    let tables = HostileTables::committed().unwrap();

    let checked = tables.check(&tasks.iter().map(String::as_str).collect::<Vec<_>>());

    assert_eq!(checked, Ok(()));
}
