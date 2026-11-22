use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::{CONFIGURED_KINDS, ConfiguredKind, EgressClass};

/// Each kind of configured destination has one row of the table, the one
/// it is classed by: every one ADR 0017 and the spec list, the Clean-up
/// rules' sources and a browser mirror.
/// Covers: story:4-16, setting:browser.mirror
#[rstest]
#[case::a_model_provider(ConfiguredKind::ModelProvider)]
#[case::a_secret_provider(ConfiguredKind::SecretProvider)]
#[case::a_proxy(ConfiguredKind::Proxy)]
#[case::a_notification_webhook(ConfiguredKind::NotificationWebhook)]
#[case::an_otlp_collector(ConfiguredKind::Otlp)]
#[case::the_plugin_index(ConfiguredKind::PluginIndex)]
#[case::a_cdp_url(ConfiguredKind::CdpUrl)]
#[case::a_clean_up_rules_source(ConfiguredKind::CleanUpRulesSource)]
#[case::a_browser_mirror(ConfiguredKind::BrowserMirror)]
fn each_configured_kind_has_one_row_of_the_table(#[case] kind: ConfiguredKind) {
    assert_eq!(
        (
            CONFIGURED_KINDS
                .iter()
                .filter(|row| row.kind == kind)
                .count(),
            kind.classed().kind
        ),
        (1, kind)
    );
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
