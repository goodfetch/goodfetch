//! One layer's values, read from settings nested by key as every layer
//! writes them (a file's table, a Run's `settings`, a Recipe's manifest):
//! each path is walked until it names a key, a name may hold several
//! segments (`"crawl.retry"`), and a key's value is checked against its
//! kind. The product check comes first (ADR 0087): a key only the other
//! Product honours is kept aside, unapplied, in a configuration file, which
//! both Products share, and refused with `not_in_product` anywhere else.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{RunOrigin, SettingsLayer};
use serde_json::{Map, Value};

use super::error::SettingsError;
use super::keys::{self, Key};

/// Who gave a value: where a run-layer value came from, and whether a
/// Client acting for a model gave it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Given {
    pub(crate) origin: Option<RunOrigin>,
    pub(crate) for_model: bool,
}

/// What one layer sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    pub(crate) applied: BTreeMap<&'static str, Value>,
    pub(crate) others: BTreeMap<&'static str, Value>,
    pub(crate) origin: Option<RunOrigin>,
    /// Whether a Client acting for a model gave them (ADR 0092).
    pub(crate) for_model: bool,
}

impl Values {
    /// Reads `nested`, the values `layer` gives, on an engine of `engine`.
    ///
    /// # Errors
    ///
    /// `setting_unknown` for a key no Product knows, `setting_invalid` for
    /// a value not of its key's kind, and, outside configuration,
    /// `not_in_product` for a key `engine` lacks.
    pub fn read(
        layer: SettingsLayer,
        nested: &Map<String, Value>,
        engine: Product,
    ) -> Result<Self, SettingsError> {
        let mut values = Self::default();
        walk(layer, "", nested, engine, &mut values)?;
        Ok(values)
    }

    /// The same, from the run layer's `origin`.
    #[must_use]
    pub fn from_origin(mut self, origin: RunOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// The same, given by a Client acting for a model, which sets no key a
    /// model may not (ADR 0092).
    #[must_use]
    pub fn from_model(mut self) -> Self {
        self.for_model = true;
        self
    }

    /// Who gave its values.
    pub(crate) fn given(&self) -> Given {
        Given {
            origin: self.origin,
            for_model: self.for_model,
        }
    }

    /// Whether it sets nothing, applied or not.
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty() && self.others.is_empty()
    }

    /// The value it sets for `key`, if it applies it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.applied.get(key)
    }

    /// The keys it sets and applies, in order.
    pub fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.applied.keys().copied()
    }
}

fn walk(
    layer: SettingsLayer,
    prefix: &str,
    table: &Map<String, Value>,
    engine: Product,
    values: &mut Values,
) -> Result<(), SettingsError> {
    for (name, value) in table {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(key) = keys::find(&path) {
            leaf(layer, key, value, engine, values)?;
        } else if let (true, Some(table)) = (keys::is_prefix(&path), value.as_object()) {
            walk(layer, &path, table, engine, values)?;
        } else {
            return Err(SettingsError::Unknown {
                closest: keys::closest(&path).map(str::to_owned),
                key: path,
                layer: Some(layer),
            });
        }
    }
    Ok(())
}

fn leaf(
    layer: SettingsLayer,
    key: Key,
    value: &Value,
    engine: Product,
    values: &mut Values,
) -> Result<(), SettingsError> {
    let name = key.setting.name;
    if !engine.contains(key.product) {
        if layer.is_configuration() {
            values.others.insert(name, value.clone());
            return Ok(());
        }
        return Err(SettingsError::NotInProduct {
            key: name.to_owned(),
            needs: key.product,
        });
    }
    let checked = if value.is_null() {
        if key.setting.clearable {
            Ok(())
        } else {
            Err("expected a value; `null` does not clear this setting".to_owned())
        }
    } else {
        key.setting.kind.check(value)
    };
    checked.map_err(|reason| SettingsError::Invalid {
        key: name.to_owned(),
        layer: Some(layer),
        reason,
    })?;
    values.applied.insert(name, value.clone());
    Ok(())
}
