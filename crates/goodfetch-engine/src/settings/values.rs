//! One layer's values, read from settings nested by key as every layer
//! writes them (a file's table, a Run's `settings`, a Recipe's manifest):
//! each path is walked until it names a key, a name may hold several
//! segments (`"crawl.retry"`), and a key's value is checked against its
//! kind, a Session source's `domains` against the Public Suffix List as
//! well (ADR 0056). The product check comes first (ADR 0087): a key only the other
//! Product honours is kept aside, unapplied, in a configuration file, which
//! both Products share, and refused with `not_in_product` anywhere else.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::secrets::placeholders::SECRETS_PLACEHOLDERS;
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{RunOrigin, SESSION_SOURCE, SettingsLayer};
use serde_json::{Map, Value};

use super::error::SettingsError;
use super::keys::{self, Key};
use crate::host_name;

/// Who gave a value: where a run-layer value came from, whether a Client
/// acting for a model gave it, and whether it has only the run layer's
/// authority wherever its layer ranks it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Given {
    pub(crate) origin: Option<RunOrigin>,
    pub(crate) for_model: bool,
    pub(crate) run_authority: bool,
}

/// What one layer sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    pub(crate) applied: BTreeMap<&'static str, Value>,
    pub(crate) others: BTreeMap<&'static str, Value>,
    pub(crate) origin: Option<RunOrigin>,
    /// Whether a Client acting for a model gave them (ADR 0092).
    pub(crate) for_model: bool,
    /// Whether they have only the run layer's authority, as a project
    /// configuration sent to an Exposed Daemon (ADR 0083).
    pub(crate) run_authority: bool,
}

impl Values {
    /// Reads `nested`, the values `layer` gives, on an engine of `engine`.
    ///
    /// # Errors
    ///
    /// `setting_unknown` for a key no Product knows, `setting_invalid` for
    /// a value not of its key's kind or a Session source domain below its
    /// registrable domain, and, outside configuration,
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

    /// The same, with only the run layer's authority while keeping its
    /// layer's rank, as a project configuration a Client sends to an
    /// Exposed Daemon: it widens nothing reserved to configuration (ADR
    /// 0083).
    #[must_use]
    pub fn with_run_authority(mut self) -> Self {
        self.run_authority = true;
        self
    }

    /// Who gave its values.
    pub(crate) fn given(&self) -> Given {
        Given {
            origin: self.origin,
            for_model: self.for_model,
            run_authority: self.run_authority,
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
    if name == SESSION_SOURCE.name
        && let Ok(source) = serde_json::from_value::<SessionSource>(value.clone())
    {
        check_session_source(&source, Some(layer))?;
    }
    values.applied.insert(name, value.clone());
    Ok(())
}

/// Checks that each of `source`'s `domains` is a registrable domain (ADR
/// 0056), an address or a host the Public Suffix List gives none: the rule
/// `session.source` meets in every layer, and a Session's own `source`
/// meets where the Session opens, `source` set in `layer`.
///
/// # Errors
///
/// [`SettingsError::NotRegistrable`] for the first domain that lies below
/// its registrable domain, naming that registrable domain.
pub fn check_session_source(
    source: &SessionSource,
    layer: Option<SettingsLayer>,
) -> Result<(), SettingsError> {
    source.domains().iter().try_for_each(|domain| {
        let Some(key) = host_name::key(domain) else {
            return Ok(());
        };
        match host_name::registrable(&key) {
            Some(registrable) if registrable != key => Err(SettingsError::NotRegistrable {
                key: SESSION_SOURCE.name.to_owned(),
                layer,
                domain: domain.clone(),
                registrable: registrable.to_owned(),
            }),
            _ => Ok(()),
        }
    })
}

/// `value`, given for `setting` by a nearer layer or scope, over `below`,
/// the value beneath it: `value` whole, but for `secrets.placeholders`,
/// which binds each placeholder on its own, nearest winning (ADR 0092),
/// so a placeholder `value` leaves out keeps `below`'s binding.
pub(crate) fn over(setting: &Setting, below: &Value, value: &Value) -> Value {
    match (below, value) {
        (Value::Object(below), Value::Object(nearer))
            if setting.name == SECRETS_PLACEHOLDERS.name =>
        {
            let mut bound = below.clone();
            bound.extend(nearer.clone());
            Value::Object(bound)
        }
        _ => value.clone(),
    }
}
