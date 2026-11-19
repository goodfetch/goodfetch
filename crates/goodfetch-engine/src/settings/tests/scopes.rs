//! A scope's own settings (ADR 0083): a key outside the scope is refused
//! with `setting_not_applicable` naming where it is set, and a Run's run
//! layer takes the Connection's values beneath its own.

use goodfetch_protocol::catalog::{Product, SettingScope};
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::super::{SettingsError, beneath_connection, check_scope};

fn map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

/// What `check_scope` refuses `value` at `scope` with: its code, key and
/// fix.
fn refused(value: Value, scope: SettingScope) -> Option<(&'static str, String, String)> {
    match check_scope(&map(value), scope, Product::Foxhound) {
        Ok(()) => None,
        Err(error) => {
            let code = error.code();
            match error {
                SettingsError::NotApplicable { key, fix, .. } => Some((code, key, fix)),
                other => panic!("unexpected refusal {other:?}"),
            }
        }
    }
}

/// Covers: story:11-70, setting:timeout.run, error:setting_not_applicable
#[test]
fn a_runs_own_timeout_run_is_refused_and_named_on_the_connection() {
    let at_run = refused(json!({ "timeout": { "run": 60_000 } }), SettingScope::Run);
    let at_connection = refused(
        json!({ "timeout": { "run": 60_000 } }),
        SettingScope::Connection,
    );

    let (code, key, fix) = at_run.unwrap();
    assert_eq!(
        (code, key.as_str(), at_connection),
        ("setting_not_applicable", "timeout.run", None)
    );
    assert!(fix.contains("connect()"), "{fix}");
}

/// A key no program's code sets passes the scope check, for the resolver
/// to refuse naming the layer and the file that set it.
///
/// Covers: story:11-70
#[rstest]
#[case::connection(SettingScope::Connection)]
#[case::run(SettingScope::Run)]
fn a_key_only_configuration_sets_is_left_to_the_resolver(#[case] scope: SettingScope) {
    assert_eq!(
        refused(json!({ "proxy": { "providers": {} } }), scope),
        None
    );
}

/// Covers: story:11-70
#[rstest]
#[case::connection(SettingScope::Connection)]
#[case::run(SettingScope::Run)]
fn a_key_the_scope_takes_passes(#[case] scope: SettingScope) {
    assert_eq!(
        refused(
            json!({ "timeout": { "navigation": 1_000 }, "content": { "extractor": "trafilatura" } }),
            scope
        ),
        None
    );
}

/// A Run's run layer: what it leaves out it inherits from its Connection,
/// its `null` clears and its value sets, and a value that is a table is
/// replaced whole, never merged field by field.
///
/// Covers: story:11-72
#[test]
fn a_run_takes_its_connections_values_beneath_its_own() {
    let connection = map(json!({
        "timeout": { "navigation": 1_000, "wait": 3_000 },
        "crawl": { "max_depth": 5, "domain_keys": { "a.test": "a", "b.test": "b" } },
    }));
    let run = map(json!({
        "timeout": { "wait": 4_000 },
        "crawl": { "max_depth": null, "domain_keys": { "c.test": "c" } },
    }));

    let layer = beneath_connection(&connection, &run, Product::Foxhound).unwrap();

    assert_eq!(
        Value::Object(layer),
        json!({
            "timeout.navigation": 1_000,
            "crawl.max_depth": null,
            "timeout.wait": 4_000,
            "crawl.domain_keys": { "c.test": "c" },
        })
    );
}

/// Covers: story:11-72
#[test]
fn a_run_on_a_connection_without_settings_keeps_its_own_as_given() {
    let run = map(json!({ "timeout": { "navigation": 1_000 } }));

    assert_eq!(
        beneath_connection(&Map::new(), &run, Product::Collie).unwrap(),
        run
    );
}
