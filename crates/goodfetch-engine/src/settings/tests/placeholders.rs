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

use super::{catalog_key, locked, nested, only, over_layers, values};
use crate::settings::{Layers, SettingsError, Values, beneath_connection, resolve};

const PLACEHOLDERS: &str = "secrets.placeholders";

/// A binding of the placeholder `aws` to the developer's AWS key.
fn aws_key() -> Value {
    json!({ "aws": { "reference": "env:AWS_SECRET_ACCESS_KEY", "domains": ["amazonaws.com"] } })
}

/// A binding of `github` to `reference`.
fn github(reference: &str) -> Value {
    json!({ "reference": reference, "domains": ["github.com"] })
}

/// The run layer's values that bind `bound`, by placeholder.
fn binding(bound: &Value) -> serde_json::Map<String, Value> {
    nested(json!({ "secrets": { "placeholders": bound } }))
}

/// Story 4-51: a binding set at the connection, overridden at a Run and
/// again at a Session, resolves to the nearest, placeholder by
/// placeholder, so a placeholder only the connection binds stays bound;
/// two Sessions of one Run each use their own reference.
/// Covers: story:4-51, setting:secrets.placeholders
#[test]
fn each_placeholder_resolves_to_its_nearest_binding() {
    let connection = binding(&json!({
        "github": github("env:CONNECTION_GITHUB"),
        "aws": aws_key()["aws"],
    }));
    let own = binding(&json!({ "github": github("env:RUN_GITHUB") }));
    let layers = Layers {
        run: Values::read(
            SettingsLayer::Run,
            &beneath_connection(&connection, &own, Product::Collie).unwrap(),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    let run = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let session = |reference: &str| {
        let own = binding(&json!({ "github": github(reference) }));
        run.beneath(&Values::read(SettingsLayer::Run, &own, Product::Collie).unwrap())
            .unwrap()
    };

    let (first, second) = (session("env:FIRST_GITHUB"), session("env:SECOND_GITHUB"));

    let setting = catalog_key(PLACEHOLDERS);
    let bound = |reference: &str| json!({ "github": github(reference), "aws": aws_key()["aws"] });
    assert_eq!(
        (
            run.value(setting),
            first.value(setting),
            second.value(setting)
        ),
        (
            &bound("env:RUN_GITHUB"),
            &bound("env:FIRST_GITHUB"),
            &bound("env:SECOND_GITHUB")
        )
    );
}

/// A binding in user configuration stays beneath a project's and a Run's
/// that bind other placeholders, and yields to the one that binds its own.
/// Covers: story:4-51, setting:secrets.placeholders
#[test]
fn configuration_and_a_run_bind_placeholder_by_placeholder() {
    let mut layers = only(SettingsLayer::UserConfiguration, PLACEHOLDERS, &aws_key());
    layers.project_configuration = values(
        SettingsLayer::ProjectConfiguration,
        json!({ PLACEHOLDERS: { "github": github("env:PROJECT_GITHUB") } }),
    );
    layers.run = values(
        SettingsLayer::Run,
        json!({ PLACEHOLDERS: { "aws": github("env:RUN_AWS") } }),
    );

    let resolved = over_layers(&layers, PLACEHOLDERS);

    assert_eq!(
        resolved,
        Ok(json!({ "github": github("env:PROJECT_GITHUB"), "aws": github("env:RUN_AWS") }))
    );
}

/// Story 4-55: a Recipe never binds a placeholder and a Client acting for
/// a model never sets one, nor `proxy.server` or the proxy's credentials,
/// which send a reference's value to a destination they also name; the
/// program's own code and configuration bind (positive controls).
/// Covers: story:4-55, error:setting_locked, setting:secrets.placeholders, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[rstest]
#[case::bound_by_the_programs_own_code(PLACEHOLDERS, aws_key(), SettingsLayer::Run, false, true)]
#[case::bound_in_user_configuration(
    PLACEHOLDERS,
    aws_key(),
    SettingsLayer::UserConfiguration,
    false,
    true
)]
#[case::bound_in_project_configuration(
    PLACEHOLDERS,
    aws_key(),
    SettingsLayer::ProjectConfiguration,
    false,
    true
)]
#[case::bound_by_a_recipe(PLACEHOLDERS, aws_key(), SettingsLayer::Recipe, false, false)]
#[case::bound_by_a_model(PLACEHOLDERS, aws_key(), SettingsLayer::Run, true, false)]
#[case::bound_in_a_models_project_configuration(
    PLACEHOLDERS,
    aws_key(),
    SettingsLayer::ProjectConfiguration,
    true,
    false
)]
#[case::proxy_server_by_a_model("proxy.server", json!("http://proxy.attacker.test:3128"), SettingsLayer::Run, true, false)]
#[case::proxy_server_by_a_recipe("proxy.server", json!("http://proxy.attacker.test:3128"), SettingsLayer::Recipe, false, false)]
#[case::proxy_username_by_a_model("proxy.username", json!("env:PROXY_USER"), SettingsLayer::Run, true, false)]
#[case::proxy_password_by_a_model("proxy.password", json!("env:PROXY_PASSWORD"), SettingsLayer::Run, true, false)]
#[case::proxy_server_by_the_programs_own_code("proxy.server", json!("http://127.0.0.1:3128"), SettingsLayer::Run, false, true)]
fn a_recipe_and_a_model_never_send_a_secret_where_they_choose(
    #[case] key: &str,
    #[case] value: Value,
    #[case] layer: SettingsLayer,
    #[case] for_model: bool,
    #[case] taken: bool,
) {
    let mut layers = only(layer, key, &value);
    if for_model {
        layers.run = layers.run.from_model();
        layers.project_configuration = layers.project_configuration.from_model();
    }

    let resolved = over_layers(&layers, key);

    let expected = if taken { Ok(value) } else { locked(key) };
    assert_eq!(resolved, expected);
}

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
