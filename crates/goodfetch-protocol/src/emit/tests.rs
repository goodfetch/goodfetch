use rstest::rstest;
use serde_json::json;

use super::emit;
use crate::catalog::Product;

#[test]
fn the_bundle_holds_the_schema_of_the_releases_file() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["files"]["compat/releases.toml"],
        json!({ "$ref": "#/$defs/ReleasesFile" })
    );
    assert_eq!(
        emitted.bundle["$defs"]["ReleasesFile"]["properties"]["release"]["items"],
        json!({ "$ref": "#/$defs/Release" })
    );
}

#[test]
fn the_bundle_registers_each_format_with_its_rule_and_schema() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["formats"],
        json!([
            {
                "name": "protocol",
                "version": "0.1",
                "storage": "transcript",
                "rule": "default",
                "schema": { "$ref": "#/$defs/Message" },
            },
            {
                "name": "run-index",
                "version": "0.1",
                "storage": "sqlite",
                "rule": "default",
                "schema": { "$ref": "#/$defs/RunIndexRow" },
            },
            {
                "name": "run-state",
                "version": "0.1",
                "storage": "sqlite",
                "rule": "default",
                "schema": { "$ref": "#/$defs/RunStateRow" },
            },
        ])
    );
}

#[test]
fn every_event_an_operation_names_is_in_the_catalog() {
    let catalog = emit().catalog;

    let listed: Vec<&str> = catalog["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["name"].as_str().unwrap())
        .collect();
    let unlisted: Vec<&str> = catalog["operations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|operation| operation["events"].as_array().unwrap())
        .map(|event| event.as_str().unwrap())
        .filter(|event| !listed.contains(event))
        .collect();
    assert_eq!(unlisted, Vec::<&str>::new());
}

#[rstest]
#[case::collie(Product::Collie, "collie")]
#[case::foxhound(Product::Foxhound, "foxhound")]
fn product_reads_and_writes_the_mark_the_catalog_writes(
    #[case] product: Product,
    #[case] mark: &str,
) {
    let written = serde_json::to_value(product).unwrap();

    assert_eq!(written, json!(mark));
    assert_eq!(product.as_str(), mark);
    assert_eq!(serde_json::from_value::<Product>(written).unwrap(), product);
}

#[test]
fn the_bundle_holds_the_schemas_of_the_result_document_and_the_case_log_entry() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["documents"],
        json!({
            "result_document": { "$ref": "#/$defs/ResultDocument" },
            "case_log_entry": { "$ref": "#/$defs/CaseLogEntry" },
        })
    );
}
