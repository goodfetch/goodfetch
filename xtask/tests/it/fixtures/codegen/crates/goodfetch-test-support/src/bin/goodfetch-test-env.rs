//! Prints the fixture's schemas, keyed by file name, with `--print-schemas`,
//! and its Secret canaries' `gitleaks` rule with `--print-canary-rule`.

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--print-schemas") => {
            let schemas = serde_json::json!({
                "test-env-description.schema.json":
                    schemars::schema_for!(goodfetch_test_support::Description),
            });
            println!("{schemas}");
        }
        Some("--print-canary-rule") => print!("{}", goodfetch_test_support::gitleaks_rule()),
        other => panic!("unexpected argument {other:?}"),
    }
}
