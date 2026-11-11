use rstest::rstest;
use serde_json::json;

use super::emit;

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
            "name": "clean-up-rules",
            "version": "0.1",
            "storage": "json",
            "rule": "newest_readable",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/CleanUpRulesManifest" },
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
        {
            "name": "known-certificates",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "frozen_major",
            "hello_from": [],
            "schema": { "$ref": "#/$defs/CertificatePin" },
        },
        {
            "name": "export",
            "version": "0.1",
            "storage": "json",
            "rule": "default",
            "hello_from": [],
            "schema": { "type": "array", "items": true },
        },
    ])
}

/// The formats of the registry that are settings files, by name.
const SETTINGS_FILES: [&str; 2] = ["operator-policy", "user-configuration"];

#[test]
fn the_bundle_registers_each_format_with_its_rule_and_schema() {
    let emitted = emit();
    let (settings_files, formats): (Vec<serde_json::Value>, Vec<serde_json::Value>) = emitted
        .bundle["formats"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .partition(|format| SETTINGS_FILES.contains(&format["name"].as_str().unwrap()));
    let settings_files: Vec<serde_json::Value> = settings_files
        .into_iter()
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
            "browser.clean": { "$ref": "#/$defs/BrowserCleaned" },
            "browser.install": { "$ref": "#/$defs/BrowserInstalled" },
            "config.explain": { "$ref": "#/$defs/SettingsExplanation" },
            "daemon.health": { "$ref": "#/$defs/DaemonHealthOutput" },
            "daemon.http-listener-token": { "$ref": "#/$defs/HttpListenerToken" },
            "daemon.install": { "$ref": "#/$defs/DaemonStatus" },
            "daemon.restart": { "$ref": "#/$defs/DaemonStatus" },
            "daemon.rotate-token": { "$ref": "#/$defs/TokenRotated" },
            "daemon.run": { "$ref": "#/$defs/DaemonStatus" },
            "daemon.start": { "$ref": "#/$defs/DaemonStatus" },
            "daemon.status": { "$ref": "#/$defs/DaemonStatus" },
            "daemon.stop": { "$ref": "#/$defs/DaemonStatus" },
            "daemon.uninstall": { "$ref": "#/$defs/DaemonUninstallOutput" },
            "runs.reindex": { "$ref": "#/$defs/RunsReindexOutput" },
            "upgrade-files": { "$ref": "#/$defs/FilesUpgraded" },
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
        json!(["code", "group", "retryable", "message"])
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

/// The name of each type the params of a request take: every operation's,
/// handler request's and Plugin method's, and `hello`'s.
fn request_params(emitted: &super::Emitted) -> Vec<String> {
    let name = |params: &serde_json::Value| {
        params["$ref"]
            .as_str()
            .and_then(|reference| reference.strip_prefix("#/$defs/"))
            .unwrap()
            .to_owned()
    };
    let catalog = &emitted.catalog;
    let entries = ["operations", "handler_requests"]
        .into_iter()
        .flat_map(|section| catalog[section].as_array().unwrap())
        .map(|entry| name(&entry["params"]));
    let methods = catalog["plugin_kinds"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|kind| kind["methods"].as_array().unwrap())
        .map(|method| name(&method["params"]));
    let hello = name(&emitted.bundle["connection"]["hello"]["params"]);
    entries.chain(methods).chain([hello]).collect()
}

/// Covers: story:3-146
#[test]
fn every_requests_params_may_hold_its_trace_context_in_meta() {
    let emitted = emit();
    let definitions = &emitted.bundle["$defs"];

    for params in request_params(&emitted) {
        assert_eq!(
            definitions[&params]["properties"]["_meta"],
            json!({ "$ref": "#/$defs/TraceContext", "arrived_in": "0.1" }),
            "{params}"
        );
    }
    assert_eq!(
        emitted.bundle["connection"]["meta"],
        json!({ "$ref": "#/$defs/TraceContext" })
    );
    let trace_context = &definitions["TraceContext"];
    assert_eq!(trace_context["additionalProperties"], json!(false));
    let mut keys: Vec<&String> = trace_context["properties"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    keys.sort();
    assert_eq!(keys, ["traceparent", "tracestate"]);
    assert_eq!(trace_context.get("required"), None);
}

/// `_meta` joins a request's params type in place, so no params type may
/// be another type's member, where it would take `_meta` too.
#[test]
fn no_requests_params_type_is_a_member_of_another_type() {
    let emitted = emit();
    let params = request_params(&emitted);
    let definitions = serde_json::to_string(&emitted.bundle["$defs"]).unwrap();
    let outputs = serde_json::to_string(&json!([
        emitted.bundle["outputs"],
        emitted.bundle["documents"],
        emitted.bundle["files"],
        emitted.bundle["formats"],
    ]))
    .unwrap();

    for name in params {
        let reference = format!("\"#/$defs/{name}\"");
        assert!(
            !definitions.contains(&reference) && !outputs.contains(&reference),
            "{name} is a member of another type"
        );
    }
}

#[test]
fn the_bundle_holds_the_block_cause_type_with_every_cause() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["shared_types"]["block_cause"],
        json!({ "$ref": "#/$defs/BlockCause" })
    );
    let mut values: Vec<&str> = emitted.bundle["$defs"]["BlockCause"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["const"].as_str().unwrap())
        .collect();
    values.sort_unstable();
    assert_eq!(
        values,
        [
            "captcha",
            "challenge",
            "consent_required",
            "decoy",
            "ip_block",
            "payment_required",
            "rate_limit"
        ]
    );
}
