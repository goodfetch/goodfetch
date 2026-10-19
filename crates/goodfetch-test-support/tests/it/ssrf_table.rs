//! The SSRF case table's committed file against its committed JSON Schema
//! (ADR 0067), the contract another language's runner reads the table by.

use serde_json::Value;

/// The committed table.
const TABLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/ssrf/cases.toml"
);

/// The table's committed schema, which `xtask codegen` writes.
const SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/ssrf-cases.schema.json"
);

/// The schema's errors for `table`, written as TOML.
fn schema_errors(table: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    let table: Value = toml::from_str(table).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&table)
        .map(|error| error.to_string())
        .collect()
}

#[test]
fn the_committed_table_validates_against_its_committed_schema() {
    let table = std::fs::read_to_string(TABLE).unwrap();

    let errors = schema_errors(&table);

    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn a_case_of_an_unknown_family_fails_the_schema() {
    let table = r#"
        [[cases]]
        name = "decimal_integer"
        family = "spelling"
        url = "http://2130706433/"
        refusal = { code = "egress_refused", rule = "private_address" }
    "#;

    let errors = schema_errors(table);

    assert_ne!(errors, Vec::<String>::new());
}
