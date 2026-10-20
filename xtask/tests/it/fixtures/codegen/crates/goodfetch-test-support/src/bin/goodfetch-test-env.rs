//! Prints the fixture's schemas, keyed by their paths under
//! `test-support/fixtures/`, with `--print-schemas`.

fn main() {
    assert_eq!(
        std::env::args().nth(1).as_deref(),
        Some("--print-schemas")
    );
    let schemas = serde_json::json!({
        "engine-scripts/script.schema.json":
            schemars::schema_for!(goodfetch_test_support::Script),
        "schemas/test-env-description.schema.json":
            schemars::schema_for!(goodfetch_test_support::Description),
    });
    println!("{schemas}");
}
