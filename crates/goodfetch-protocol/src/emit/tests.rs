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

/// The formats of the registry that are no settings file, as the bundle
/// writes them.
fn file_formats() -> serde_json::Value {
    json!([
        {
            "name": "protocol",
            "version": "0.1",
            "storage": "transcript",
            "rule": "default",
            "hello_from": ["client", "browser_host"],
            "schema": { "$ref": "#/$defs/Message" },
        },
        {
            "name": "browser-cache",
            "version": "0.1",
            "storage": "json",
            "rule": "default",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/BrowserCacheEntry" },
        },
        {
            "name": "run-index",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "shared_store",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/RunIndexRow" },
        },
        {
            "name": "run-state",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "default",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/RunStateRow" },
        },
        {
            "name": "installed-executables",
            "version": "0.1",
            "storage": "json",
            "rule": "frozen_major",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/InstalledExecutables" },
        },
        {
            "name": "usage-ledger",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "frozen_major",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/Limit" },
        },
        {
            "name": "politeness-file",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "frozen_major",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/PermitRow" },
        },
        {
            "name": "audit-log",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "frozen_major",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/AuditEntry" },
        },
    ])
}

#[test]
fn the_bundle_registers_each_format_with_its_rule_and_schema() {
    let emitted = emit();
    let mut formats = emitted.bundle["formats"].as_array().unwrap().clone();
    let settings_files: Vec<serde_json::Value> = formats
        .drain(8..)
        .map(|format| {
            json!({
                "name": format["name"],
                "version": format["version"],
                "storage": format["storage"],
                "rule": format["rule"],
                "hello_from": format["hello_from"],
                "tables": format["schema"]["properties"].as_object().unwrap().keys().collect::<Vec<_>>(),
            })
        })
        .collect();

    assert_eq!(
        settings_files,
        [
            json!({
                "name": "operator-policy",
                "version": "0.1",
                "storage": "toml",
                "rule": "shared_store",
                "hello_from": [],
                "tables": ["cap", "lock", "lowest_minor", "managed", "settings", "version"],
            }),
            json!({
                "name": "user-configuration",
                "version": "0.1",
                "storage": "toml",
                "rule": "shared_store",
                "hello_from": [],
                "tables": ["lowest_minor", "settings", "version"],
            }),
        ]
    );
    assert_eq!(serde_json::Value::Array(formats), file_formats());
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

/// Covers: story:3-86, story:3-142, setting:network.resolve
#[rstest]
#[case::a_key_collie_carries("network.resolve", json!(["collie", "foxhound"]), "every")]
#[case::a_key_only_foxhound_carries("crawl.retry.max", json!(["foxhound"]), "every")]
#[case::a_key_only_the_machine_sets(
    "browser_cache.max_unused_age",
    json!(["collie", "foxhound"]),
    "machine"
)]
fn the_catalog_records_each_settings_products_from_its_mark_and_its_authority(
    #[case] key: &str,
    #[case] products: serde_json::Value,
    #[case] authority: &str,
) {
    let catalog = emit().catalog;

    let entry = catalog["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["name"] == key)
        .unwrap();

    assert_eq!(
        (&entry["products"], entry["authority"].as_str()),
        (&products, Some(authority))
    );
}

#[test]
fn a_settings_entry_carries_its_values_schema_and_default() {
    let catalog = emit().catalog;

    let entry = catalog["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["name"] == "timeout.navigation")
        .unwrap();

    assert_eq!(
        (
            &entry["value"],
            &entry["default"],
            &entry["clearable"],
            &entry["weakens"]
        ),
        (
            &json!({ "type": "integer", "minimum": 0, "description": "A duration in milliseconds." }),
            &json!(30_000),
            &json!(false),
            &json!(false),
        )
    );
}

#[test]
fn a_settings_files_schema_nests_each_key_by_its_segments() {
    let emitted = emit();
    let policy = emitted.bundle["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|format| format["name"] == "operator-policy")
        .unwrap();

    let resolve =
        &policy["schema"]["properties"]["lock"]["properties"]["network"]["properties"]["resolve"];

    assert_eq!(
        (&resolve["type"], &resolve["additionalProperties"]["items"]),
        (
            &json!("object"),
            &json!({ "type": "string", "format": "ip" })
        )
    );
}

/// Each command's `--json` output is a type of the bundle (ADR 0069),
/// under the command's path.
#[test]
fn the_bundle_holds_each_commands_json_output() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["outputs"],
        json!({
            "browsers.clean": { "$ref": "#/$defs/BrowsersCleanOutput" },
            "browsers.install": { "$ref": "#/$defs/BrowsersInstallOutput" },
            "config.explain": { "$ref": "#/$defs/SettingsExplanation" },
            "daemon.install": { "$ref": "#/$defs/DaemonInstallOutput" },
            "daemon.restart": { "$ref": "#/$defs/DaemonStartOutput" },
            "daemon.rotate-token": { "$ref": "#/$defs/DaemonRotateTokenOutput" },
            "daemon.start": { "$ref": "#/$defs/DaemonStartOutput" },
            "daemon.status": { "$ref": "#/$defs/DaemonStatusOutput" },
            "daemon.stop": { "$ref": "#/$defs/DaemonStopOutput" },
            "daemon.uninstall": { "$ref": "#/$defs/DaemonUninstallOutput" },
            "runs.reindex": { "$ref": "#/$defs/RunsReindexOutput" },
            "upgrade-files": { "$ref": "#/$defs/UpgradeFilesOutput" },
        })
    );
}

#[rstest]
#[case::round_trip("sample.echo", "round_trip")]
#[case::opener("run.start", "opener")]
#[case::sequence("events.subscribe", "sequence")]
fn the_catalog_declares_each_operations_kind(#[case] operation: &str, #[case] kind: &str) {
    let catalog = emit().catalog;

    let entry = catalog["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == operation)
        .unwrap();
    assert_eq!(entry["kind"], json!(kind));
}

#[rstest]
#[case::engine_info("engine.info")]
#[case::page_body("page.body")]
fn the_catalog_declares_each_operations_reach(#[case] operation: &str) {
    let catalog = emit().catalog;

    let entry = catalog["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == operation)
        .unwrap();
    assert_eq!(entry["reach"], json!("all_surfaces"));
}

#[test]
fn the_bundle_holds_the_type_a_failed_command_prints_under_json() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["command_failed"],
        json!({ "$ref": "#/$defs/CommandFailed" })
    );
    assert_eq!(
        emitted.bundle["$defs"]["CommandError"]["required"],
        json!(["code", "group", "retryable", "message", "operation"])
    );
}

#[rstest]
#[case::success("success", 0)]
#[case::a_groups_row("refused", 4)]
#[case::an_error_codes_own_row("limit_reached", 16)]
#[case::a_stopped_run("run_stopped", 20)]
#[case::sigterm("terminated", 143)]
fn the_catalog_lists_the_exit_code_tables_rows(#[case] name: &str, #[case] code: u64) {
    let emitted = emit();

    let row = emitted.catalog["exit_codes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .map(|row| (row["code"].clone(), row["product"].clone()));

    assert_eq!(row, Some((json!(code), json!("collie"))));
}
