//! The settings a program's code gives the scopes it opens (ADR 0083):
//! one [`Settings`] type for every scope, a Run's start, a Session's
//! opening and a page's, filled with the typed [`Setting`] constants
//! `xtask codegen` writes from the catalog, one a key, in modules that
//! mirror its dotted name:
//!
//! ```
//! use std::time::Duration;
//!
//! use goodfetch_collie_client::settings::{Settings, content, timeout};
//!
//! let settings = Settings::new()
//!     .set(timeout::NAVIGATION, Duration::from_secs(60))
//!     .set(content::EXTRACTOR, content::Extractor::Trafilatura);
//! ```
//!
//! A key left out inherits, `None` clears a setting that can be cleared,
//! and a value sets it. A value of the wrong type for its key does not
//! compile; a key the scope does not set fails the call that takes it with
//! `setting_not_applicable`, before anything is sent, its `fix` naming the
//! scopes that set it. Durations travel in whole milliseconds, rounded up;
//! every other value goes as given, and the engine checks it.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::settings::{
    SETTING_NOT_APPLICABLE, SettingNotApplicableDetails, SettingsLayer,
};
use serde_json::{Map, Value};

use crate::call::wire_millis;
use crate::error::Error;

include!("generated/settings.rs");

/// A key of the catalog's settings whose values are `T`: one of the
/// constants of this module's submodules, `timeout::NAVIGATION` for
/// `timeout.navigation`. A setting that can be cleared takes an `Option`,
/// whose `None` sends null.
pub struct Setting<T> {
    name: &'static str,
    scopes: &'static [SettingScope],
    value: PhantomData<fn(T)>,
}

impl<T> Setting<T> {
    /// The setting `name`, which the settings of `scopes` set.
    pub(crate) const fn new(name: &'static str, scopes: &'static [SettingScope]) -> Self {
        Self {
            name,
            scopes,
            value: PhantomData,
        }
    }

    /// Its key, as the catalog and the wire name it: `timeout.navigation`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

impl<T> Clone for Setting<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Setting<T> {}

impl<T> fmt::Debug for Setting<T> {
    /// Its key alone.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Setting").field(&self.name).finish()
    }
}

/// The settings a program gives one scope it opens (ADR 0083), by key:
/// what it leaves out inherits from the scope around it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    values: BTreeMap<&'static str, Given>,
}

/// A value a program gave, and the scopes whose settings set its key.
#[derive(Clone, Debug, PartialEq)]
struct Given {
    scopes: &'static [SettingScope],
    value: Value,
}

impl Settings {
    /// No settings: every key inherits.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `setting` to `value`, in place of any value given it before;
    /// `None` clears a setting that can be cleared.
    #[must_use]
    pub fn set<T: SettingValue>(mut self, setting: Setting<T>, value: impl Into<T>) -> Self {
        self.values.insert(
            setting.name,
            Given {
                scopes: setting.scopes,
                value: value.into().into_wire(),
            },
        );
        self
    }

    /// The settings as `scope`'s `settings` member carries them, nested by
    /// the keys' segments.
    ///
    /// # Errors
    ///
    /// `setting_not_applicable` for the first key, in key order, that the
    /// settings of `scope` do not set, its `fix` naming those that do.
    pub(crate) fn within(self, scope: SettingScope) -> Result<Map<String, Value>, Error> {
        let mut nested = Map::new();
        for (key, given) in self.values {
            if !given.scopes.contains(&scope) {
                return Err(not_applicable(key, scope, given.scopes));
            }
            let mut segments: Vec<&str> = key.split('.').collect();
            let last = segments.pop().unwrap_or(key);
            let mut level = &mut nested;
            for segment in segments {
                level = level
                    .entry(segment)
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .expect(
                        "invariant: no key of the catalog holds another, as `xtask codegen` checks",
                    );
            }
            level.insert(last.to_owned(), given.value);
        }
        Ok(nested)
    }
}

/// What a scope's settings are called in a message: the scope's opening.
fn opening(scope: SettingScope) -> &'static str {
    match scope {
        SettingScope::Connection => "the connection",
        SettingScope::Run => "a Run's start",
        SettingScope::Label => "a crawl's Label",
        SettingScope::Session => "a Session's opening",
        SettingScope::Page => "a page's opening",
    }
}

/// The `setting_not_applicable` of `key` given to `scope`, whose fix names
/// the openings of `scopes`, Collie's own: a crawl's Label is Foxhound's.
fn not_applicable(key: &str, scope: SettingScope, scopes: &[SettingScope]) -> Error {
    let openings: Vec<&str> = scopes
        .iter()
        .filter(|scope| **scope != SettingScope::Label)
        .map(|scope| opening(*scope))
        .collect();
    let fix = match openings.split_last() {
        None => "set it in configuration or an Operator policy".to_owned(),
        Some((last, [])) => format!("set it in the settings of {last}"),
        Some((last, others)) => {
            format!("set it in the settings of {} or {last}", others.join(", "))
        }
    };
    let details = SettingNotApplicableDetails {
        key: key.to_owned(),
        layer: SettingsLayer::Run,
        fix: fix.clone(),
    };
    let details: Map<String, Value> = serde_json::to_value(details)
        .and_then(serde_json::from_value)
        .expect("invariant: an Error code's details serialize as an object");
    Error::answered(ErrorObject {
        code: SETTING_NOT_APPLICABLE.group.json_rpc_class(),
        message: format!("`{key}` is no setting of {}: {fix}.", opening(scope)),
        data: ErrorData {
            code: SETTING_NOT_APPLICABLE.name.to_owned(),
            retryable: SETTING_NOT_APPLICABLE.retryable,
            details,
        },
    })
}

/// A type a [`Setting`]'s values take: `bool`, `u64`, `String`, a
/// [`Duration`], a choice's own enum, a `Vec` or a `BTreeMap` of them, a
/// JSON value for a shape the module does not type, and an `Option` of
/// any for a setting that can be cleared.
pub trait SettingValue: sealed::Wire {}

/// What turns a value into the JSON the wire carries, which only this
/// crate implements.
mod sealed {
    use serde_json::Value;

    /// A value as the wire carries it.
    pub trait Wire {
        /// The value's JSON.
        fn into_wire(self) -> Value;
    }
}

impl sealed::Wire for bool {
    fn into_wire(self) -> Value {
        Value::Bool(self)
    }
}

impl SettingValue for bool {}

impl sealed::Wire for u64 {
    fn into_wire(self) -> Value {
        Value::from(self)
    }
}

impl SettingValue for u64 {}

impl sealed::Wire for String {
    fn into_wire(self) -> Value {
        Value::String(self)
    }
}

impl SettingValue for String {}

/// Whole milliseconds, rounded up, so that no duration but zero sends 0.
impl sealed::Wire for Duration {
    fn into_wire(self) -> Value {
        Value::from(wire_millis(self))
    }
}

impl SettingValue for Duration {}

/// As given, for a shape the module does not type.
impl sealed::Wire for Value {
    fn into_wire(self) -> Value {
        self
    }
}

impl SettingValue for Value {}

/// Null for `None`.
impl<T: SettingValue> sealed::Wire for Option<T> {
    fn into_wire(self) -> Value {
        self.map_or(Value::Null, sealed::Wire::into_wire)
    }
}

impl<T: SettingValue> SettingValue for Option<T> {}

impl<T: SettingValue> sealed::Wire for Vec<T> {
    fn into_wire(self) -> Value {
        Value::Array(self.into_iter().map(sealed::Wire::into_wire).collect())
    }
}

impl<T: SettingValue> SettingValue for Vec<T> {}

impl<T: SettingValue> sealed::Wire for BTreeMap<String, T> {
    fn into_wire(self) -> Value {
        Value::Object(
            self.into_iter()
                .map(|(name, entry)| (name, entry.into_wire()))
                .collect(),
        )
    }
}

impl<T: SettingValue> SettingValue for BTreeMap<String, T> {}

#[cfg(test)]
mod tests;
