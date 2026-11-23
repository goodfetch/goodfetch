//! Who binds a Secret placeholder, and what a binding's `domains` take
//! (ADR 0092, spec #564's issue 654).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::{Value, json};

use super::nested;
use crate::settings::{SettingsError, Values};

const PLACEHOLDERS: &str = "secrets.placeholders";

/// A binding's `domains` lists registrable domains only: a host below one
/// is refused with `setting_invalid` naming the registrable domain to
/// write, and without `domains` too, from any layer.
/// Covers: error:setting_invalid, setting:secrets.placeholders
#[rstest]
#[case::a_subdomain_from_the_run(SettingsLayer::Run, json!(["www.example.com"]), "`example.com`")]
#[case::a_subdomain_from_user_configuration(SettingsLayer::UserConfiguration, json!(["www.example.com"]), "`example.com`")]
#[case::no_domains_from_the_run(SettingsLayer::Run, Value::Null, "`domains`")]
fn a_binding_to_no_registrable_domain_is_refused_with_setting_invalid(
    #[case] layer: SettingsLayer,
    #[case] domains: Value,
    #[case] names: &str,
) {
    let mut bound = json!({ "reference": "env:EXAMPLE_PASSWORD" });
    if !domains.is_null() {
        bound["domains"] = domains;
    }
    let set = json!({ PLACEHOLDERS: { "example": bound } });

    let refused = Values::read(layer, &nested(set), Product::Collie).unwrap_err();

    let SettingsError::Invalid {
        key,
        layer: refused_in,
        ..
    } = &refused
    else {
        panic!("not setting_invalid: {refused:?}");
    };
    let shown = refused.to_string();
    assert_eq!(
        (key.as_str(), *refused_in, shown.contains(names)),
        (PLACEHOLDERS, Some(layer), true),
        "{shown}"
    );
}
