//! Prints the fixture's schemas, keyed by file name, with `--print-schemas`.

fn main() {
    assert_eq!(
        std::env::args().nth(1).as_deref(),
        Some("--print-schemas")
    );
    let schemas = serde_json::json!({
        "test-env-description.schema.json":
            schemars::schema_for!(goodfetch_test_support::Description),
    });
    println!("{schemas}");
}
