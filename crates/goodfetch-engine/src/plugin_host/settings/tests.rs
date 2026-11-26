use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{InvalidSettings, PluginSettings, given};
use crate::settings::{Layers, Values, resolve};

/// A Plugin `scripted` whose manifest declares `schema`, if any.
fn manifest(schema: Option<Value>) -> PluginManifest {
    let mut manifest = json!({
        "manifest_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
    });
    if let Some(schema) = schema {
        manifest["settings_schema"] = schema;
    }
    PluginManifest::read(manifest.to_string().as_bytes()).unwrap()
}

/// A schema with a `language` of two letters, `fr` by default, a
/// `depth` from 1, a secret `token`, and a nested `auth` whose `key` is
/// secret too.
fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "language": { "type": "string", "pattern": "^[a-z]{2}$", "default": "fr" },
            "depth": { "type": "integer", "minimum": 1 },
            "token": { "type": "string", "secret": true },
            "auth": {
                "type": "object",
                "properties": { "key": { "type": "string", "secret": true } },
            },
        },
    })
}

fn checked(schema: Option<Value>, given: Value) -> Result<PluginSettings, InvalidSettings> {
    let Value::Object(given) = given else {
        panic!("settings are an object");
    };
    PluginSettings::checked(
        "scripted",
        &manifest(schema),
        given,
        Some(SettingsLayer::Run),
    )
}

/// The key and reason of a refusal.
fn refused(refusal: Result<PluginSettings, InvalidSettings>) -> (String, String) {
    let invalid = refusal.unwrap_err();
    assert_eq!(invalid.layer, Some(SettingsLayer::Run));
    (invalid.key, invalid.reason)
}

/// A field left out takes the schema's default, and a Secret reference in
/// a secret field is taken as written.
///
/// Covers: story:7-79
#[test]
fn a_field_left_out_takes_the_schemas_default_and_a_secret_field_takes_a_reference() {
    let settings = checked(
        Some(schema()),
        json!({ "token": "env:TOKEN", "auth": { "key": "env:KEY" } }),
    )
    .unwrap();

    assert_eq!(
        settings.values,
        Map::from_iter([
            ("language".to_owned(), json!("fr")),
            ("token".to_owned(), json!("env:TOKEN")),
            ("auth".to_owned(), json!({ "key": "env:KEY" })),
        ])
    );
}

/// A value the schema refuses, or a value given in a secret field's place,
/// is `setting_invalid` naming the field's key.
///
/// Covers: story:7-79
#[rstest]
#[case::wrong_pattern(json!({ "language": "french" }), "plugins.scripted.settings.language")]
#[case::below_minimum(json!({ "depth": 0 }), "plugins.scripted.settings.depth")]
#[case::unknown_field(json!({ "colour": "red" }), "plugins.scripted.settings")]
#[case::literal_secret(json!({ "token": "hunter22" }), "plugins.scripted.settings.token")]
#[case::nested_literal_secret(json!({ "auth": { "key": "hunter22" } }), "plugins.scripted.settings.auth.key")]
fn a_value_the_schema_refuses_is_setting_invalid_naming_its_key(
    #[case] given: Value,
    #[case] key: &str,
) {
    let (refused_key, reason) = refused(checked(Some(schema()), given));

    assert_eq!(refused_key, key, "{reason}");
}

/// A literal in a secret field is refused, naming the field and never
/// repeating the value.
///
/// Covers: story:7-79
#[test]
fn a_literal_in_a_secret_field_is_refused_without_its_value() {
    let (key, reason) = refused(checked(Some(schema()), json!({ "token": "hunter22" })));

    assert_eq!(
        (key.as_str(), reason.contains("hunter22")),
        ("plugins.scripted.settings.token", false),
        "{reason}"
    );
}

/// A schema that marks a field secret where `properties` alone do not reach
/// it refuses the Plugin's settings whole, so no value passes for a
/// reference unchecked; a `"secret": true` held as data, or a field named
/// `secret`, marks nothing.
///
/// Covers: story:7-79
#[rstest]
#[case::under_all_of(
    json!({ "allOf": [{ "properties": { "token": { "type": "string", "secret": true } } }] }),
    false
)]
#[case::under_defs(
    json!({
        "$defs": { "token": { "type": "string", "secret": true } },
        "properties": { "token": { "$ref": "#/$defs/token" } },
    }),
    false
)]
#[case::under_items(
    json!({ "properties": { "tokens": { "type": "array", "items": { "type": "string", "secret": true } } } }),
    false
)]
#[case::held_as_data(
    json!({ "properties": { "shape": { "type": "object", "default": { "secret": true } } } }),
    true
)]
#[case::a_field_named_secret(json!({ "properties": { "secret": true } }), true)]
fn a_secret_field_out_of_the_engines_reach_refuses_the_settings_whole(
    #[case] schema: Value,
    #[case] taken: bool,
) {
    let answer = checked(Some(schema), json!({}))
        .map(drop)
        .map_err(|invalid| invalid.key);

    assert_eq!(
        answer,
        if taken {
            Ok(())
        } else {
            Err("plugins.scripted.settings".to_owned())
        }
    );
}

/// A Plugin's settings name the layer that set its own entry of `plugins`,
/// not the nearest layer that set another Plugin's.
///
/// Covers: story:7-79
#[test]
fn a_plugins_settings_name_the_layer_that_set_its_entry() {
    let values = |layer, value: Value| {
        let Value::Object(nested) = value else {
            panic!("settings are an object");
        };
        Values::read(layer, &nested, Product::Collie).unwrap()
    };
    let layers = Layers {
        user_configuration: values(
            SettingsLayer::UserConfiguration,
            json!({ "plugins": { "kettle": { "settings": { "depth": 0 } } } }),
        ),
        run: values(
            SettingsLayer::Run,
            json!({ "plugins": { "scripted": { "settings": {} } } }),
        ),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    assert_eq!(
        ["kettle", "scripted", "absent"].map(|name| given(Some(&resolved), name).1),
        [
            Some(SettingsLayer::UserConfiguration),
            Some(SettingsLayer::Run),
            None
        ]
    );
}

/// A Plugin with no schema takes no settings, and a schema that does not
/// compile refuses its settings whole.
///
/// Covers: story:7-79
#[test]
fn a_plugin_without_a_schema_takes_none_and_a_broken_schema_refuses_its_settings() {
    let none = checked(None, json!({})).unwrap();
    let given = refused(checked(None, json!({ "language": "fr" })));
    let broken = refused(checked(Some(json!({ "type": 12 })), json!({})));

    assert_eq!(
        (none, given.0, broken.0),
        (
            PluginSettings::default(),
            "plugins.scripted.settings".to_owned(),
            "plugins.scripted.settings".to_owned()
        )
    );
}
