use rstest::rstest;
use schemars::JsonSchema;
use serde_json::Value;

use super::all;

/// Every value `T` takes on the wire, as its JSON Schema lists them: the
/// variants its serde attributes name, read apart from any hand-kept list,
/// in order.
pub(crate) fn every_value_of<T: JsonSchema>() -> Vec<String> {
    let schema = schemars::schema_for!(T);
    let schema = schema.as_value();
    let listed = schema["enum"].as_array().into_iter().flatten();
    let one_of = schema["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|variant| &variant["const"]);
    let mut values: Vec<String> = listed
        .chain(one_of)
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    values.sort();
    values
}

/// `values`, in order.
pub(crate) fn sorted(values: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
    values.sort();
    values
}

/// A wire enum's `ALL` holds each of its values once, each named as serde
/// sends it.
#[rstest]
#[case::artifact_kind(sorted(&all(&crate::browser_cache::ArtifactKind::ALL)), every_value_of::<crate::browser_cache::ArtifactKind>())]
#[case::error_group(sorted(&all(&crate::catalog::ErrorGroup::ALL)), every_value_of::<crate::catalog::ErrorGroup>())]
#[case::product(sorted(&all(&crate::catalog::Product::ALL)), every_value_of::<crate::catalog::Product>())]
#[case::rules_source(sorted(&all(&crate::clean_up_rules::RulesSource::ALL)), every_value_of::<crate::clean_up_rules::RulesSource>())]
#[case::surface(sorted(&all(&crate::connection::Surface::ALL)), every_value_of::<crate::connection::Surface>())]
#[case::egress_rule(sorted(&all(&crate::egress::EgressRule::ALL)), every_value_of::<crate::egress::EgressRule>())]
#[case::hello_from(sorted(&all(&crate::formats::HelloFrom::ALL)), every_value_of::<crate::formats::HelloFrom>())]
#[case::rule(sorted(&all(&crate::formats::Rule::ALL)), every_value_of::<crate::formats::Rule>())]
#[case::pressure_source(sorted(&all(&crate::governor::PressureSource::ALL)), every_value_of::<crate::governor::PressureSource>())]
#[case::work_kind(sorted(&all(&crate::governor::WorkKind::ALL)), every_value_of::<crate::governor::WorkKind>())]
#[case::log_level(sorted(&all(&crate::log::LogLevel::ALL)), every_value_of::<crate::log::LogLevel>())]
#[case::case_outcome(sorted(&all(&crate::result_document::CaseOutcome::ALL)), every_value_of::<crate::result_document::CaseOutcome>())]
#[case::journey_outcome(sorted(&all(&crate::result_document::JourneyOutcome::ALL)), every_value_of::<crate::result_document::JourneyOutcome>())]
#[case::oracle_outcome(sorted(&all(&crate::result_document::OracleOutcome::ALL)), every_value_of::<crate::result_document::OracleOutcome>())]
#[case::outcome(sorted(&all(&crate::result_document::Outcome::ALL)), every_value_of::<crate::result_document::Outcome>())]
#[case::completeness(sorted(&all(&crate::run::Completeness::ALL)), every_value_of::<crate::run::Completeness>())]
#[case::lifecycle(sorted(&all(&crate::run::Lifecycle::ALL)), every_value_of::<crate::run::Lifecycle>())]
#[case::period_length(sorted(&all(&crate::run::PeriodLength::ALL)), every_value_of::<crate::run::PeriodLength>())]
#[case::presence(sorted(&all(&crate::run::Presence::ALL)), every_value_of::<crate::run::Presence>())]
#[case::run_kind(sorted(&all(&crate::run::RunKind::ALL)), every_value_of::<crate::run::RunKind>())]
#[case::verdict(sorted(&all(&crate::run::Verdict::ALL)), every_value_of::<crate::run::Verdict>())]
#[case::run_trigger(sorted(&all(&crate::run::RunTrigger::ALL)), every_value_of::<crate::run::RunTrigger>())]
#[case::limit_setter(sorted(&all(&crate::run::LimitSetter::ALL)), every_value_of::<crate::run::LimitSetter>())]
#[case::secret_failure(sorted(&all(&crate::secrets::SecretFailure::ALL)), every_value_of::<crate::secrets::SecretFailure>())]
#[case::secret_input_refused_reason(sorted(&all(&crate::secrets::SecretInputRefusedReason::ALL)), every_value_of::<crate::secrets::SecretInputRefusedReason>())]
#[case::session_export_failure(sorted(&all(&crate::secrets::SessionExportFailure::ALL)), every_value_of::<crate::secrets::SessionExportFailure>())]
#[case::run_origin(sorted(&all(&crate::settings::RunOrigin::ALL)), every_value_of::<crate::settings::RunOrigin>())]
#[case::settings_layer(sorted(&all(&crate::settings::SettingsLayer::ALL)), every_value_of::<crate::settings::SettingsLayer>())]
#[case::policy_table(sorted(&all(&crate::settings::edit::PolicyTable::ALL)), every_value_of::<crate::settings::edit::PolicyTable>())]
#[case::sampled_folder(sorted(&all(&crate::storage::SampledFolder::ALL)), every_value_of::<crate::storage::SampledFolder>())]
#[case::storage_level(sorted(&all(&crate::storage::StorageLevel::ALL)), every_value_of::<crate::storage::StorageLevel>())]
#[case::listener(sorted(&all(&crate::audit::Listener::ALL)), every_value_of::<crate::audit::Listener>())]
#[case::refusal_reason(sorted(&all(&crate::audit::RefusalReason::ALL)), every_value_of::<crate::audit::RefusalReason>())]
#[case::known_certificate_failure(sorted(&all(&crate::tls::KnownCertificateFailure::ALL)), every_value_of::<crate::tls::KnownCertificateFailure>())]
fn a_wire_enums_values_are_every_value_it_sends(
    #[case] all: Vec<String>,
    #[case] every_value: Vec<String>,
) {
    assert_eq!(all, every_value);
}
