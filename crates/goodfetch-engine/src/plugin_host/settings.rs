//! A Plugin's settings (ADR 0033, issue 861): the Plugin's entry of the
//! `plugins` setting, checked against the JSON Schema of its manifest's
//! `settings_schema` as its instance starts, each `default` the schema
//! gives filled in where the entry leaves a field out; and the variables
//! of its environment its manifest's `env` declares, each valued from the
//! field it names.
//!
//! A field the schema marks `"secret": true` ([`SECRET_KEYWORD`]) takes
//! only a Secret reference: a value given in its place is refused, so a
//! secret is never written into a settings layer. The engine resolves the
//! reference only to value a variable, and only for a Plugin whose
//! manifest's `permissions.grants` holds `secret_value` and whose Operator
//! policy entry approves that reference's value (issue 900), the value
//! then joining the Run's mask as any resolved Secret does and its release
//! recorded in the Audit log.

use std::collections::BTreeMap;

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::ValueKind;
use goodfetch_protocol::plugin::approvals::{
    Approvals, Origin as PluginOrigin, PLUGIN_SECRET_VALUE_RELEASED, Permission,
};
use goodfetch_protocol::plugin::settings::{PLUGINS, SECRET_KEYWORD};
use goodfetch_protocol::plugin::{Grant, PluginManifest};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value, json};

use crate::audit::Origin;
use crate::call_error::CallError;
use crate::secrets::{Purpose, RunSecrets, SecretError};
use crate::settings::{Resolved, SettingsError};

/// A Plugin's settings as given: its entry's `settings` in the `plugins`
/// of `resolved`, a Run's settings, and the layer that set that entry.
pub fn given(
    resolved: Option<&Resolved>,
    name: &str,
) -> (Map<String, Value>, Option<SettingsLayer>) {
    let Some(resolved) = resolved else {
        return (Map::new(), None);
    };
    let settings = resolved.value(&PLUGINS)[name]["settings"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    (settings, resolved.entry_layer(&PLUGINS, name))
}

/// The key of the field at `path` of the Plugin `name`'s settings, as
/// `plugins.<name>.settings.<path>`, or of its settings whole.
fn key(name: &str, path: &[String]) -> String {
    let mut key = format!("{}.{name}.settings", PLUGINS.name);
    for segment in path {
        key.push('.');
        key.push_str(segment);
    }
    key
}

/// A Plugin's settings, checked against its manifest's schema.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginSettings {
    /// Each field, every default the schema gives filled in.
    values: Map<String, Value>,
    /// The path of each field the schema marks secret.
    secret: Vec<Vec<String>>,
}

impl PluginSettings {
    /// `given`, the settings of the Plugin `name` from `layer`, checked
    /// against `manifest`'s `settings_schema`, each `default` it gives
    /// filled in.
    ///
    /// # Errors
    ///
    /// `setting_invalid` naming the field's key for a value the schema
    /// refuses or a value given in a secret field's place, and naming the
    /// settings whole for any setting of a Plugin with no schema, a schema
    /// that does not compile, or one that marks a field secret where
    /// `properties` alone do not reach it.
    pub fn checked(
        name: &str,
        manifest: &PluginManifest,
        given: Map<String, Value>,
        layer: Option<SettingsLayer>,
    ) -> Result<Self, InvalidSettings> {
        let invalid = |path: &[String], reason: String| InvalidSettings {
            key: key(name, path),
            layer,
            reason,
        };
        let Some(schema) = &manifest.settings_schema else {
            if given.is_empty() {
                return Ok(Self::default());
            }
            return Err(invalid(
                &[],
                "the Plugin's manifest declares no `settings_schema`, so it takes no settings"
                    .to_owned(),
            ));
        };
        let mut values = given;
        with_defaults(schema, &mut values);
        let mut secret = Vec::new();
        secret_fields(schema, &mut Vec::new(), &mut secret);
        if secret_marks(schema) > secret.len() {
            return Err(invalid(
                &[],
                format!(
                    "the Plugin's `settings_schema` marks a field `\"{SECRET_KEYWORD}\": true` that `properties` alone do not reach, so the engine cannot check that it holds a Secret reference"
                ),
            ));
        }
        for path in &secret {
            if let Some(value) = at(&values, path) {
                ValueKind::SecretReference
                    .check(value)
                    .map_err(|reason| invalid(path, reason))?;
            }
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "a Plugin's settings schema compiles once as its instance starts in a Run, on no Shared path (ADR 0140)"
        )]
        let validator = jsonschema::validator_for(schema).map_err(|error| {
            invalid(
                &[],
                format!("the Plugin's `settings_schema` is no valid JSON Schema: {error}"),
            )
        })?;
        let instance = Value::Object(values);
        if let Err(error) = validator.validate(&instance) {
            let path: Vec<String> = error
                .instance_path()
                .segments()
                .map(|segment| segment.to_string())
                .collect();
            return Err(invalid(&path, error.to_string()));
        }
        let Value::Object(values) = instance else {
            unreachable!("invariant: the instance was built from an object");
        };
        Ok(Self { values, secret })
    }

    /// The validated settings map, with schema defaults applied.
    #[must_use]
    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }

    /// The paths of fields the schema marks `"secret": true`.
    #[must_use]
    pub fn secret_paths(&self) -> &[Vec<String>] {
        &self.secret
    }

    /// The variables of the Plugin's environment `manifest`'s `env`
    /// declares, each valued from the field it names: a string as it is,
    /// another value as its JSON text, and none for a field left out or
    /// `null`. A secret field's reference is resolved by `secrets` for the
    /// Run `run_id`, under its key, from `layer`, for a first-party Plugin by
    /// default (so it receives cloud credentials as Secrets from the engine,
    /// ADR 0033) and for a third-party or Private Plugin only when the
    /// Operator policy's `approved` covers its value; each release is recorded
    /// in the Audit log as `plugin_secret_value_released`.
    ///
    /// # Errors
    ///
    /// [`EnvironmentError`]'s.
    pub async fn environment(
        &self,
        name: &str,
        (manifest, approved, plugin_origin): (&PluginManifest, &Approvals, PluginOrigin),
        secrets: &RunSecrets,
        (run_id, layer): (&RunId, Option<SettingsLayer>),
    ) -> Result<BTreeMap<String, String>, EnvironmentError> {
        let mut environment = BTreeMap::new();
        for (variable, field) in &manifest.env {
            let path: Vec<String> = field.split('.').map(str::to_owned).collect();
            let Some(value) = at(&self.values, &path).filter(|value| !value.is_null()) else {
                continue;
            };
            let text = if self.secret.contains(&path) {
                if !manifest.permissions.grants.contains(&Grant::SecretValue) {
                    return Err(EnvironmentError::Ungranted {
                        variable: variable.clone(),
                        field: field.clone(),
                    });
                }
                let reference = value
                    .as_str()
                    .and_then(|text| SecretReference::parse(text).ok())
                    .expect("invariant: a secret field holds a Secret reference once checked");
                let released = Permission::SecretValue {
                    reference: reference.as_str().to_owned(),
                };
                if plugin_origin != PluginOrigin::FirstParty && !approved.covers(&released) {
                    return Err(EnvironmentError::Denied(released));
                }
                let placeholder = key(name, &path);
                let purpose = Purpose {
                    placeholder: &placeholder,
                    domain: None,
                    domains: &[],
                    layer,
                    run_id: Some(run_id),
                };
                let text = secrets
                    .resolve(&reference, &purpose)
                    .await
                    .map_err(EnvironmentError::Secret)?
                    .expose_secret()
                    .to_owned();
                let origin = Origin {
                    actor: Actor::Engine,
                    surface: None,
                    layer,
                    run_id: Some(run_id.clone()),
                };
                let details = json!({ "plugin": name, "reference": reference.as_str() });
                secrets
                    .audit(&origin, &PLUGIN_SECRET_VALUE_RELEASED, details)
                    .await
                    .map_err(|source| {
                        EnvironmentError::Secret(SecretError::Unrecorded {
                            placeholder,
                            reference,
                            source,
                        })
                    })?;
                text
            } else {
                match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                }
            };
            environment.insert(variable.clone(), text);
        }
        Ok(environment)
    }
}

/// A Plugin's settings its manifest's schema refuses: `setting_invalid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidSettings {
    /// The field's key, as `plugins.<name>.settings.<field>`, or the
    /// settings' whole.
    pub key: String,
    /// The layer that set the Plugin's entry of `plugins`.
    pub layer: Option<SettingsLayer>,
    /// What is wrong.
    pub reason: String,
}

impl From<InvalidSettings> for CallError {
    /// `setting_invalid`, refused as the engine uses the value rather than
    /// as a call gives it. Its key is below `plugins`, in no catalog, so a
    /// Client reads it as written (ADR 0099).
    fn from(InvalidSettings { key, layer, reason }: InvalidSettings) -> Self {
        let message = (SettingsError::Invalid {
            key: key.clone(),
            layer,
            reason: reason.clone(),
        })
        .to_string();
        let details = SettingInvalidDetails {
            key,
            layer,
            reason,
            limit: None,
            fix: None,
        };
        Self::new(&SETTING_INVALID, message, &details)
    }
}

/// Why a Plugin's environment cannot be valued.
#[derive(Debug, thiserror::Error)]
pub enum EnvironmentError {
    /// A secret field values a variable of a Plugin whose manifest does not
    /// ask for a Secret's value: the Plugin cannot start.
    #[error(
        "its manifest values `{variable}` from the secret field `{field}` but its `permissions.grants` lacks `secret_value`"
    )]
    Ungranted {
        /// The variable.
        variable: String,
        /// The field.
        field: String,
    },
    /// The Operator policy has not approved the release of a secret field's
    /// value to the Plugin: `plugin_permission_denied`.
    #[error("the Operator policy has not approved its permission {0:?}")]
    Denied(Permission),
    /// A secret field's reference gave no value, or its release could not
    /// be recorded.
    #[error(transparent)]
    Secret(SecretError),
}

/// The value at `path` in `values`, if any.
fn at<'value>(values: &'value Map<String, Value>, path: &[String]) -> Option<&'value Value> {
    let (first, rest) = path.split_first()?;
    rest.iter()
        .try_fold(values.get(first)?, |value, segment| value.get(segment))
}

/// `values` with each `default` `schema`'s `properties` give for a field
/// they leave out, in nested objects too.
fn with_defaults(schema: &Value, values: &mut Map<String, Value>) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (field, property) in properties {
        if !values.contains_key(field)
            && let Some(default) = property.get("default")
        {
            values.insert(field.clone(), default.clone());
        }
        if let Some(Value::Object(nested)) = values.get_mut(field) {
            with_defaults(property, nested);
        }
    }
}

/// Adds to `secret` the path of each field below `path` that `schema`'s
/// `properties` mark secret, in nested objects too.
fn secret_fields(schema: &Value, path: &mut Vec<String>, secret: &mut Vec<Vec<String>>) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (field, property) in properties {
        path.push(field.clone());
        if property.get(SECRET_KEYWORD) == Some(&Value::Bool(true)) {
            secret.push(path.clone());
        } else {
            secret_fields(property, path, secret);
        }
        path.pop();
    }
}

/// The keywords whose value is data, never a schema.
const DATA_KEYWORDS: [&str; 4] = ["default", "const", "enum", "examples"];

/// The keywords whose value maps names, never keywords, to schemas.
const SCHEMA_MAPS: [&str; 5] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];

/// How many schemas within `schema` mark a field secret, wherever they
/// sit: more than [`secret_fields`] finds means some sit where it does not
/// look.
fn secret_marks(schema: &Value) -> usize {
    match schema {
        Value::Object(keywords) => {
            let own = usize::from(keywords.get(SECRET_KEYWORD) == Some(&Value::Bool(true)));
            keywords
                .iter()
                .filter(|(keyword, _)| !DATA_KEYWORDS.contains(&keyword.as_str()))
                .map(|(keyword, value)| match value {
                    Value::Object(named) if SCHEMA_MAPS.contains(&keyword.as_str()) => {
                        named.values().map(secret_marks).sum()
                    }
                    _ => secret_marks(value),
                })
                .sum::<usize>()
                + own
        }
        Value::Array(schemas) => schemas.iter().map(secret_marks).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
