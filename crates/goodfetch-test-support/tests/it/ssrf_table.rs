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

/// The committed layers.
const LAYERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/ssrf/layers.toml"
);

/// The layers' committed schema, which `xtask codegen` writes.
const LAYERS_SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/ssrf-layers.schema.json"
);

#[test]
fn the_committed_layers_validate_against_their_committed_schema() {
    let schema: Value =
        serde_json::from_str(&std::fs::read_to_string(LAYERS_SCHEMA).unwrap()).unwrap();
    let layers: Value = toml::from_str(&std::fs::read_to_string(LAYERS).unwrap()).unwrap();

    let errors: Vec<String> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&layers)
        .map(|error| error.to_string())
        .collect();

    assert_eq!(errors, Vec::<String>::new());
}

/// Every layer Collie carries runs the table, each family every layer
/// meets or the reason it never does, and every runner the layers name is
/// a test of its file that takes its cases by its name (ADR 0067).
///
/// Covers: story:4-89
#[test]
fn every_layer_collie_carries_has_runners_for_every_family_of_the_ssrf_table() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let layers = goodfetch_test_support::SsrfLayers::committed().unwrap();
    let table = goodfetch_test_support::SsrfTable::committed().unwrap();

    let result = layers.check(&table, |path| std::fs::read_to_string(root.join(path)).ok());

    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}
