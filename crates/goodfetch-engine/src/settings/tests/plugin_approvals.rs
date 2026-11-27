//! Who decides whether a Plugin loads and what it may do: only an Operator
//! policy sets `plugins.<name>.allow`, `.digest` and `.approved` (ADRs 0033
//! and 0035, issue 900), and a nearer layer's entry for the Plugin keeps
//! them.

use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::json;

use super::{catalog_key, only, policy, refusal, resolved, values};
use crate::settings::{Layers, SettingsError};

/// An Operator policy's decision on a Plugin holds under a nearer layer's
/// entry for it, which sets only the Plugin's `settings`.
///
/// Covers: story:7-16, story:7-17
#[test]
fn the_decision_of_the_operator_policy_holds_under_a_nearer_entry() {
    let decided = json!({
        "allow": true,
        "digest": "sha256:11",
        "approved": { "effects": ["shop.test"] },
    });
    let layers = Layers {
        operator_policy: policy(
            json!({ "plugins": { "kettle": decided.clone() } }),
            json!({}),
            json!({}),
        ),
        project_configuration: values(
            SettingsLayer::ProjectConfiguration,
            json!({ "plugins": { "kettle": { "settings": { "language": "fr" } } } }),
        ),
        ..Layers::default()
    };

    let resolved = resolved(&layers);

    let mut expected = decided;
    expected["settings"] = json!({ "language": "fr" });
    assert_eq!(
        resolved.value(catalog_key("plugins")),
        &json!({ "kettle": expected })
    );
}

/// `allow`, `digest` and `approved` set anywhere but an Operator policy
/// are refused with `setting_not_applicable`, naming the Operator policy.
///
/// Covers: story:7-16
#[rstest]
fn only_an_operator_policy_decides_on_a_plugin(
    #[values(
        SettingsLayer::UserConfiguration,
        SettingsLayer::ProjectConfiguration,
        SettingsLayer::Recipe,
        SettingsLayer::Run
    )]
    layer: SettingsLayer,
    #[values(
        json!({ "allow": true }),
        json!({ "digest": "sha256:11" }),
        json!({ "approved": { "direct": true } })
    )]
    entry: serde_json::Value,
) {
    let refused = refusal(&only(layer, "plugins", &json!({ "kettle": entry })));

    let SettingsError::NotApplicable { fix, .. } = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(refused.code(), "setting_not_applicable");
    assert!(fix.contains("Operator policy"), "{fix}");
}

/// An Operator policy sets them.
#[test]
fn an_operator_policy_decides_on_a_plugin() {
    let layers = only(
        SettingsLayer::OperatorPolicy,
        "plugins",
        &json!({ "kettle": { "allow": false } }),
    );

    assert_eq!(
        resolved(&layers).value(catalog_key("plugins")),
        &json!({ "kettle": { "allow": false } })
    );
}
