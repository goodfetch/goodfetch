use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{InvalidSettings, PluginSettings};

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

/// A secret field's refusal says a value is never taken in a reference's
/// place, and never repeats the value.
///
/// Covers: story:7-79
#[test]
fn a_literal_in_a_secret_field_is_refused_without_its_value() {
    let (_, reason) = refused(checked(Some(schema()), json!({ "token": "hunter22" })));

    assert_eq!(
        (
            reason.contains("a value is never taken in its place"),
            reason.contains("hunter22")
        ),
        (true, false),
        "{reason}"
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
