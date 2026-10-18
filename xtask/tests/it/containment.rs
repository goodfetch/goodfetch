//! The containment check (ADR 0094): every generated contract of Collie's is
//! in Foxhound's, unchanged, read from the projections `xtask codegen`
//! commits.
//! Covers: story:1-121

use rstest::rstest;
use serde_json::Value;

use crate::workspace::{Workspace, finding};

const FOXHOUND: &str = "crates/goodfetch-protocol/generated/foxhound.json";

/// The codegen fixture, whose committed projections Foxhound's contains
/// Collie's, with Foxhound's projection changed by `change`.
fn with_foxhound_projection(change: impl FnOnce(&mut Value)) -> Workspace {
    let workspace = Workspace::from_fixture("codegen");
    let mut projection: Value = serde_json::from_str(&workspace.read_text(FOXHOUND)).unwrap();
    change(&mut projection);
    workspace.write(
        FOXHOUND,
        serde_json::to_string_pretty(&projection)
            .unwrap()
            .as_bytes(),
    );
    workspace
}

fn entry<'a>(projection: &'a mut Value, section: &str, name: &str) -> &'a mut Value {
    projection[section]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["name"] == name)
        .unwrap()
}

#[test]
fn foxhound_carrying_every_collie_contract_unchanged_passes() {
    let workspace = Workspace::from_fixture("codegen");

    let outcome = workspace.xtask(&["containment"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::operation("operations", "sample.echo")]
#[case::event("events", "sample.echoed")]
#[case::error_code("error_codes", "sample_refused")]
#[case::plugin_kind("plugin_kinds", "sample_store")]
fn collie_entry_missing_from_foxhound_fails(#[case] section: &str, #[case] name: &str) {
    let workspace = with_foxhound_projection(|projection| {
        projection[section]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["name"] != name);
    });

    let outcome = workspace.xtask(&["containment"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "missing-from-foxhound",
            &format!("{section}/{name}")
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn collie_entry_changed_in_foxhound_fails() {
    let workspace = with_foxhound_projection(|projection| {
        entry(projection, "operations", "sample.echo")["cancellable"] = Value::Bool(false);
    });

    let outcome = workspace.xtask(&["containment"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("changed-in-foxhound", "operations/sample.echo")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn collie_type_changed_in_foxhound_fails() {
    let workspace = with_foxhound_projection(|projection| {
        projection["$defs"]["SampleEchoParams"]["required"] = Value::Array(vec![]);
    });

    let outcome = workspace.xtask(&["containment"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("changed-in-foxhound", "$defs/SampleEchoParams")]
    );
    assert_eq!(outcome.code, 1);
}
