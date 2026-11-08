use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::{CONFIGURED_KINDS, ConfiguredKind, EgressClass};

/// Each kind of configured destination, by the name the table gives it:
/// every one ADR 0017 and the spec list, and the Clean-up rules' sources.
/// Covers: story:4-16
#[rstest]
#[case::a_model_provider(ConfiguredKind::ModelProvider, "Model provider")]
#[case::a_secret_provider(ConfiguredKind::SecretProvider, "Secret provider")]
#[case::a_proxy(ConfiguredKind::Proxy, "proxy")]
#[case::a_notification_webhook(ConfiguredKind::NotificationWebhook, "Notification webhook")]
#[case::an_otlp_collector(ConfiguredKind::Otlp, "OTLP")]
#[case::the_plugin_index(ConfiguredKind::PluginIndex, "Plugin index")]
#[case::a_cdp_url(ConfiguredKind::CdpUrl, "cdp_url")]
#[case::a_clean_up_rules_source(ConfiguredKind::CleanUpRulesSource, "Clean-up rules source")]
fn each_configured_kind_has_one_row_of_the_table(#[case] kind: ConfiguredKind, #[case] name: &str) {
    let rows: Vec<&str> = CONFIGURED_KINDS
        .iter()
        .filter(|row| row.kind == kind)
        .map(|row| row.name)
        .collect();

    assert_eq!((rows, kind.classed().name), (vec![name], name));
}

/// A configured destination is an Operator-configured endpoint when
/// configuration sets it, and a `cdp_url` a run gives is an address a page
/// can reach (ADR 0056).
/// Covers: story:4-16
#[rstest]
#[case::otlp_from_the_operator_policy(
    ConfiguredKind::Otlp,
    SettingsLayer::OperatorPolicy,
    EgressClass::OperatorConfigured
)]
#[case::a_model_provider_from_user_configuration(
    ConfiguredKind::ModelProvider,
    SettingsLayer::UserConfiguration,
    EgressClass::OperatorConfigured
)]
#[case::a_cdp_url_from_project_configuration(
    ConfiguredKind::CdpUrl,
    SettingsLayer::ProjectConfiguration,
    EgressClass::OperatorConfigured
)]
#[case::a_cdp_url_from_the_run(
    ConfiguredKind::CdpUrl,
    SettingsLayer::Run,
    EgressClass::PageReachable
)]
#[case::a_cdp_url_from_a_recipe(
    ConfiguredKind::CdpUrl,
    SettingsLayer::Recipe,
    EgressClass::PageReachable
)]
fn a_configured_destination_is_classed_by_the_layer_that_set_it(
    #[case] kind: ConfiguredKind,
    #[case] layer: SettingsLayer,
    #[case] class: EgressClass,
) {
    assert_eq!(kind.classed().class(layer), class);
}
