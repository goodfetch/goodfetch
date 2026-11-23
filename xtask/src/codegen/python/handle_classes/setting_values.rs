//! A member's parameter that takes a setting's value, as `run.new_session`'s
//! `source` takes `session.source`'s for one Session: its schema
//! references, alone or beside `null`, a definition that is the setting's
//! value once its own references are written out. The member takes the
//! settings module's classes, as `settings` does, and validates the value
//! into the model.

use anyhow::Result;
use serde_json::{Map, Value};

use super::super::super::layer::Layer;
use super::super::settings::value_classes;
use super::Imports;

/// A parameter that takes a setting's value.
pub(super) struct SettingValue {
    /// The settings module's classes the value is one of.
    classes: Vec<String>,
    /// The model the value is validated into.
    model: String,
    /// Whether the parameter also admits null, which is passed as is.
    nullable: bool,
}

impl SettingValue {
    /// The annotation of the parameter `name`, and the expression that
    /// passes it to the model, importing its classes.
    pub(super) fn spell(self, name: &str, imports: &mut Imports) -> (String, String) {
        let annotation = self.classes.join(" | ");
        imports.setting_values.extend(self.classes);
        let model = self.model;
        if self.nullable {
            (
                format!("{annotation} | None"),
                format!("None if {name} is None else {model}.model_validate({name})"),
            )
        } else {
            (annotation, format!("{model}.model_validate({name})"))
        }
    }
}

/// `schema`'s setting value, when it is a reference, alone or beside
/// `null`, to a definition that is one of `layer`'s settings' values.
pub(super) fn setting_value(
    schema: &Value,
    layer: &Layer,
    imports: &mut Imports,
) -> Result<Option<SettingValue>> {
    let (reference, nullable) = match schema["anyOf"].as_array() {
        Some(variants) => {
            let mut values = variants.iter().filter(|variant| variant["type"] != "null");
            let (Some(only), None) = (values.next(), values.next()) else {
                return Ok(None);
            };
            (only, variants.len() > 1)
        }
        None => (schema, false),
    };
    let Some(model) = reference["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
    else {
        return Ok(None);
    };
    let Some(definition) = layer.definitions.get(model) else {
        return Ok(None);
    };
    let written_out = written_out(definition, &layer.definitions);
    let Some(setting) = layer
        .settings
        .iter()
        .find(|setting| setting.value == written_out)
    else {
        return Ok(None);
    };
    Ok(Some(SettingValue {
        classes: value_classes(setting)?,
        model: imports.model(model),
        nullable,
    }))
}

/// `schema` with each reference to a definition replaced by that
/// definition, the referencing schema's own keywords over it, as the
/// catalog writes a setting's value.
fn written_out(schema: &Value, definitions: &Map<String, Value>) -> Value {
    match schema {
        Value::Object(object) => {
            let referenced = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .and_then(|name| definitions.get(name));
            let mut written = match referenced {
                Some(definition) => match written_out(definition, definitions) {
                    Value::Object(definition) => definition,
                    _ => Map::new(),
                },
                None => Map::new(),
            };
            for (keyword, value) in object {
                if keyword == "$ref" && referenced.is_some() {
                    continue;
                }
                written.insert(keyword.clone(), written_out(value, definitions));
            }
            Value::Object(written)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| written_out(item, definitions))
                .collect(),
        ),
        other => other.clone(),
    }
}
