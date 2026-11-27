//! Politeness precondition for a multi-member Daemon group (ADRs 0053 and
//! 0055): a group of more than one Group member is refused unless a Politeness
//! store (`politeness.store`) is configured or the Operator policy sets
//! `politeness.sharing: per_daemon`.

use goodfetch_protocol::group::GROUP_POLITENESS_UNSHARED;
use goodfetch_protocol::politeness::POLITENESS_SHARING;

use crate::call_error::CallError;
use crate::group::operations::internal;
use crate::settings::{Policy, Resolved, Settings};

/// Checks whether the Lead Daemon's machine settings permit admitting a second
/// (or further) Group member.
///
/// # Errors
///
/// `group_politeness_unshared` when neither `politeness.store` nor
/// `politeness.sharing = "per_daemon"` in the Operator policy is effective.
pub(super) async fn check_politeness_for_group(settings: &Settings) -> Result<(), CallError> {
    let snapshot = settings.snapshot().await;
    let resolved = snapshot.resolved().map_err(|error| internal(&error))?;
    let layers = snapshot.layers().map_err(|error| internal(&error))?;
    if allows_multi_member(resolved, &layers.operator_policy) {
        return Ok(());
    }
    Err(CallError::new(
        &GROUP_POLITENESS_UNSHARED,
        "A Daemon group of more than one Group member requires `politeness.store` or `politeness.sharing = \"per_daemon\"` in the Operator policy.",
        &(),
    ))
}

/// Pure check on resolved machine settings and Operator policy (ADR 0053):
/// true when either `politeness.store` is configured (non-null) or the
/// Operator policy sets/locks `politeness.sharing = "per_daemon"` and the
/// effective `politeness.sharing` remains `"per_daemon"`.
fn allows_multi_member(resolved: &Resolved, policy: &Policy) -> bool {
    let has_store = resolved
        .effective_named("politeness.store")
        .is_some_and(|effective| !effective.value.is_null());
    let policy_allows_per_daemon = policy
        .lock
        .get(POLITENESS_SHARING.name)
        .or_else(|| policy.settings.get(POLITENESS_SHARING.name))
        .and_then(serde_json::Value::as_str)
        == Some("per_daemon");
    let effective_per_daemon = resolved
        .effective(&POLITENESS_SHARING)
        .is_some_and(|effective| effective.value.as_str() == Some("per_daemon"));
    has_store || (policy_allows_per_daemon && effective_per_daemon)
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::settings::SettingsLayer;
    use serde_json::{Value, json};

    use super::allows_multi_member;
    use crate::settings::{Layers, Policy, Values, resolve};

    fn values(layer: SettingsLayer, body: &Value) -> Values {
        Values::read(layer, body.as_object().unwrap(), Product::Collie).unwrap()
    }

    #[test]
    fn default_settings_refuse_a_multi_member_group() {
        let layers = Layers::default();
        let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
        assert!(!allows_multi_member(&resolved, &layers.operator_policy));
    }

    #[test]
    fn user_configuration_per_daemon_is_refused() {
        let layers = Layers {
            user_configuration: values(
                SettingsLayer::UserConfiguration,
                &json!({ "politeness": { "sharing": "per_daemon" } }),
            ),
            ..Layers::default()
        };
        let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
        assert!(!allows_multi_member(&resolved, &layers.operator_policy));
    }

    #[test]
    fn operator_policy_per_daemon_is_accepted_even_when_user_config_repeats_it() {
        let layers = Layers {
            operator_policy: Policy {
                settings: values(
                    SettingsLayer::OperatorPolicy,
                    &json!({ "politeness": { "sharing": "per_daemon" } }),
                ),
                ..Policy::default()
            },
            user_configuration: values(
                SettingsLayer::UserConfiguration,
                &json!({ "politeness": { "sharing": "per_daemon" } }),
            ),
            ..Layers::default()
        };
        let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
        assert!(allows_multi_member(&resolved, &layers.operator_policy));
    }

    #[test]
    fn operator_policy_per_daemon_overridden_to_shared_without_store_is_refused() {
        let layers = Layers {
            operator_policy: Policy {
                settings: values(
                    SettingsLayer::OperatorPolicy,
                    &json!({ "politeness": { "sharing": "per_daemon" } }),
                ),
                ..Policy::default()
            },
            user_configuration: values(
                SettingsLayer::UserConfiguration,
                &json!({ "politeness": { "sharing": "shared" } }),
            ),
            ..Layers::default()
        };
        let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
        assert!(!allows_multi_member(&resolved, &layers.operator_policy));
    }

    #[test]
    fn configured_politeness_store_is_accepted() {
        let layers = Layers {
            user_configuration: values(
                SettingsLayer::UserConfiguration,
                &json!({
                    "politeness": {
                        "store": {
                            "namespace": "group-test",
                            "addressing": { "kind": "server", "url": "redis://127.0.0.1:6379" }
                        }
                    }
                }),
            ),
            ..Layers::default()
        };
        let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
        assert!(allows_multi_member(&resolved, &layers.operator_policy));
    }
}
