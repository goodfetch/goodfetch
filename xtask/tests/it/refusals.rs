//! `xtask compat refusals` (ADR 0066): the refusal cases generated from the
//! format registry of the committed bundle, one file per case, among them
//! the Product check of every pair of Products (ADR 0095), the Browser
//! host's lower minor (ADR 0096) and the shared stores' lowest minor (ADR
//! 0098); and `xtask codegen --check`, which holds them to the registry.

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const BUNDLE: &str = "crates/goodfetch-protocol/generated/bundle.json";

const REFUSALS: &str = "crates/goodfetch-protocol/generated/refusals";

fn refusals(workspace: &Workspace) {
    let outcome = workspace.xtask(&["compat", "refusals"]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// One line per written case, by its path under the refusals folder: the
/// rule, the versions read, and for a `hello` who sends it to which engine
/// and the reply.
fn written_cases(workspace: &Workspace) -> Vec<String> {
    let mut lines = Vec::new();
    let root = workspace.path().join(REFUSALS);
    for format in std::fs::read_dir(&root).unwrap() {
        let format = format.unwrap();
        for case in std::fs::read_dir(format.path()).unwrap() {
            let path = case.unwrap().path();
            let case: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
            lines.push(format!("{relative}: {}", summary(&case)));
        }
    }
    lines.sort();
    lines
}

fn summary(case: &Value) -> String {
    let text = |value: &Value| value.as_str().unwrap().to_owned();
    let mut parts = vec![format!(
        "{} {} reads {}",
        text(&case["rule"]),
        text(&case["reader"]),
        text(&case["found"])
    )];
    if let Some(lowest_minor) = case.get("lowest_minor") {
        parts.push(format!(
            "marked {lowest_minor} for an engine at {}",
            text(&case["engine"])
        ));
    }
    parts.push(format!("= {}", text(&case["compatibility"])));
    let mut line = parts.join(" ");
    if let Some(hello) = case.get("hello") {
        let mut parts = vec![format!(
            "; {} to {}",
            text(&hello["from"]),
            text(&hello["engine"])
        )];
        if let Some(needs) = hello.get("needs") {
            parts.push(format!("needing {}", text(needs)));
        }
        if let Some(session_needs) = hello.get("session_needs") {
            parts.push(format!("for a Session needing {}", text(session_needs)));
        }
        parts.push(
            match (
                hello["reply"].get("accepted"),
                hello["reply"].get("refused"),
            ) {
                (Some(accepted), None) => {
                    format!("-> accepted at {}", text(&accepted["protocol_version"]))
                }
                (None, Some(refused)) => {
                    format!("-> refused with {}", text(&refused["error_code"]))
                }
                _ => panic!("no reply: {hello}"),
            },
        );
        line.push_str(&parts.join(" "));
    }
    line
}

fn sorted(lines: &[&str]) -> Vec<String> {
    let mut lines: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
    lines.sort();
    lines
}

#[test]
fn a_sample_registry_gives_each_formats_cases_under_its_rule() {
    let workspace = Workspace::from_fixture("refusals");

    refusals(&workspace);

    assert_eq!(
        written_cases(&workspace),
        sorted(&[
            "sample-file/newer-minor.json: default 1.4 reads 1.5 = newer_minor",
            "sample-file/older-major.json: default 1.4 reads 0.0 = older_major",
            "sample-file/newer-major.json: default 1.4 reads 2.0 = newer_major",
            "sample-ledger/newer-minor.json: frozen_major 0.2 reads 0.3 = compatible",
            "sample-ledger/newer-major.json: frozen_major 0.2 reads 1.0 = newer_major",
            "sample-store/newer-minor.json: shared_store 1.2 reads 1.3 = compatible",
            "sample-store/lowest-minor-met.json: shared_store 1.2 reads 1.2 marked 1 for an engine at 1.1 = compatible",
            "sample-store/lowest-minor-newer.json: shared_store 1.2 reads 1.2 marked 2 for an engine at 1.1 = newer_minor",
            "sample-store/older-major.json: shared_store 1.2 reads 0.0 = older_major",
            "sample-store/newer-major.json: shared_store 1.2 reads 2.0 = newer_major",
            "sample-protocol/client-newer-minor.json: default 2.3 reads 2.4 = newer_minor; client to collie needing collie -> refused with protocol_version_unsupported",
            "sample-protocol/client-older-major.json: default 2.3 reads 1.0 = older_major; client to collie needing collie -> refused with protocol_version_unsupported",
            "sample-protocol/client-newer-major.json: default 2.3 reads 3.0 = newer_major; client to collie needing collie -> refused with protocol_version_unsupported",
            "sample-protocol/client-needing-collie-on-a-collie-engine.json: default 2.3 reads 2.3 = compatible; client to collie needing collie -> accepted at 2.3",
            "sample-protocol/client-needing-foxhound-on-a-collie-engine.json: default 2.3 reads 2.3 = compatible; client to collie needing foxhound -> refused with not_in_product",
            "sample-protocol/client-needing-foxhound-on-a-collie-engine-at-newer-major.json: default 2.3 reads 3.0 = newer_major; client to collie needing foxhound -> refused with not_in_product",
            "sample-protocol/client-needing-collie-on-a-foxhound-engine.json: default 2.3 reads 2.3 = compatible; client to foxhound needing collie -> accepted at 2.3",
            "sample-protocol/client-needing-foxhound-on-a-foxhound-engine.json: default 2.3 reads 2.3 = compatible; client to foxhound needing foxhound -> accepted at 2.3",
            "sample-protocol/browser-host-older-minor.json: lower_minor 2.3 reads 2.2 = compatible; browser_host to collie -> accepted at 2.2",
            "sample-protocol/browser-host-newer-minor.json: lower_minor 2.3 reads 2.4 = compatible; browser_host to collie -> accepted at 2.3",
            "sample-protocol/browser-host-older-major.json: lower_minor 2.3 reads 1.0 = older_major; browser_host to collie -> refused with protocol_version_unsupported",
            "sample-protocol/browser-host-newer-major.json: lower_minor 2.3 reads 3.0 = newer_major; browser_host to collie -> refused with protocol_version_unsupported",
            "sample-protocol/browser-host-outdated.json: lower_minor 2.3 reads 2.2 = compatible; browser_host to collie for a Session needing 2.3 -> refused with browser_host_outdated",
            "sample-protocol/group-member-of-collie-joining-a-collie-group.json: default 2.3 reads 2.3 = compatible; group_member to collie needing collie -> accepted at 2.3",
            "sample-protocol/group-member-of-foxhound-joining-a-collie-group.json: default 2.3 reads 2.3 = compatible; group_member to collie needing foxhound -> refused with not_in_product",
            "sample-protocol/group-member-of-collie-joining-a-foxhound-group.json: default 2.3 reads 2.3 = compatible; group_member to foxhound needing collie -> refused with not_in_product",
            "sample-protocol/group-member-of-foxhound-joining-a-foxhound-group.json: default 2.3 reads 2.3 = compatible; group_member to foxhound needing foxhound -> accepted at 2.3",
        ])
    );
}

#[test]
fn each_case_is_a_file_of_its_own_with_the_format_it_fabricates() {
    let workspace = Workspace::from_fixture("refusals");

    refusals(&workspace);

    let written = workspace.read_text(&format!(
        "{REFUSALS}/sample-protocol/client-needing-foxhound-on-a-collie-engine.json"
    ));
    assert_eq!(
        serde_json::from_str::<Value>(&written).unwrap(),
        json!({
            "format": "sample-protocol",
            "rule": "default",
            "reader": "2.3",
            "found": "2.3",
            "compatibility": "compatible",
            "hello": {
                "from": "client",
                "engine": "collie",
                "needs": "foxhound",
                "reply": { "refused": { "error_code": "not_in_product" } },
            },
        })
    );
}

fn register(workspace: &Workspace, format: &Value) {
    let mut bundle: Value = serde_json::from_str(&workspace.read_text(BUNDLE)).unwrap();
    bundle["formats"]
        .as_array_mut()
        .unwrap()
        .push(format.clone());
    workspace.write(BUNDLE, &serde_json::to_vec_pretty(&bundle).unwrap());
}

#[test]
fn registering_a_format_adds_its_cases() {
    let workspace = Workspace::from_fixture("refusals");
    refusals(&workspace);
    let before = written_cases(&workspace);
    register(
        &workspace,
        &json!({
            "name": "sample-plan",
            "version": "0.1",
            "storage": "json",
            "rule": "default",
            "hello_from": [],
            "schema": { "type": "object" },
        }),
    );

    refusals(&workspace);

    let added: Vec<String> = written_cases(&workspace)
        .into_iter()
        .filter(|line| !before.contains(line))
        .collect();
    assert_eq!(
        added,
        [
            "sample-plan/newer-major.json: default 0.1 reads 1.0 = newer_major",
            "sample-plan/newer-preview.json: default 0.1 reads 0.2 = newer_major",
            "sample-plan/older-preview.json: default 0.1 reads 0.0 = older_major",
        ]
    );
}

#[test]
fn unregistering_a_format_removes_its_cases() {
    let workspace = Workspace::from_fixture("refusals");
    refusals(&workspace);
    let mut bundle: Value = serde_json::from_str(&workspace.read_text(BUNDLE)).unwrap();
    bundle["formats"]
        .as_array_mut()
        .unwrap()
        .retain(|format| format["name"] != "sample-file");
    workspace.write(BUNDLE, &serde_json::to_vec_pretty(&bundle).unwrap());

    refusals(&workspace);

    assert!(!workspace.path().join(REFUSALS).join("sample-file").exists());
}

/// Every rule the protocol knows has a refusal table, so a registry can
/// only miss one by naming a rule the protocol does not know: the command
/// then reaches no verdict, exits 2 and names the rule.
#[test]
fn a_format_under_a_rule_the_protocol_does_not_know_reaches_no_verdict() {
    let workspace = Workspace::from_fixture("refusals");
    register(
        &workspace,
        &json!({
            "name": "sample-response-cache",
            "version": "0.1",
            "storage": "sqlite",
            "rule": "response_cache",
            "hello_from": [],
            "schema": { "type": "object" },
        }),
    );

    let outcome = workspace.xtask(&["compat", "refusals"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("`response_cache`"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn check_fails_after_a_hand_edit_of_a_refusal_case() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        "crates/goodfetch-protocol/generated/refusals/sample-store/newer-preview.json",
        "newer_major",
        "compatible",
    );

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "drift",
            "crates/goodfetch-protocol/generated/refusals/sample-store/newer-preview.json"
        )],
        "{}",
        outcome.stderr
    );
    assert_ne!(outcome.code, 0);
}
