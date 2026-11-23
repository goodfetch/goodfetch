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
//! and a value sets it: a whole number as a bare literal, `"text"` for
//! text, and the protocol's own type for a shape it defines once, as
//! `session::PAGE_CHECK`'s `PageCheck`. A value of the wrong type for its
//! key does not compile; a key the scope does not set fails the call that takes it with
//! `setting_not_applicable`, before anything is sent, its `fix` naming the
//! scopes that set it. Durations travel in whole milliseconds, rounded up;
//! every other value goes as given, and the engine checks it.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use goodfetch_protocol::catalog::{SettingJson, SettingScope};
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::settings::{
    SETTING_NOT_APPLICABLE, SettingNotApplicableDetails, SettingsLayer,
};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::markers;

include!("generated/settings.rs");

/// A key of the catalog's settings whose values are `T`: one of the
/// constants of this module's submodules, `timeout::NAVIGATION` for
/// `timeout.navigation`. A setting that can be cleared takes an `Option`,
/// whose `None` sends null. Setting a deprecated one warns once per
/// process through `tracing` (ADR 0099).
pub struct Setting<T> {
    name: &'static str,
    scopes: &'static [SettingScope],
    /// Its deprecation's one text, when it is deprecated.
    deprecation: Option<&'static str>,
    value: PhantomData<fn(T)>,
}

// Public for Foxhound's crate's generated keys alone (ADR 0084), which
// mirror these; a program's code uses the constants.
impl<T> Setting<T> {
    /// The setting `name`, which the settings of `scopes` set.
    #[doc(hidden)]
    #[must_use]
    pub const fn new(name: &'static str, scopes: &'static [SettingScope]) -> Self {
        Self {
            name,
            scopes,
            deprecation: None,
            value: PhantomData,
        }
    }

    /// The deprecated setting `name`, which the settings of `scopes` set,
    /// with its deprecation's one text.
    #[doc(hidden)]
    #[must_use]
    pub const fn deprecated(
        name: &'static str,
        scopes: &'static [SettingScope],
        deprecation: &'static str,
    ) -> Self {
        Self {
            name,
            scopes,
            deprecation: Some(deprecation),
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
    /// `None` clears a setting that can be cleared. Setting a deprecated
    /// one warns once per process (ADR 0099).
    #[must_use]
    pub fn set<T: SettingValue>(
        mut self,
        setting: Setting<T>,
        value: impl IntoSettingValue<T>,
    ) -> Self {
        if let Some(deprecation) = setting.deprecation {
            markers::warn_setting(setting.name, deprecation);
        }
        self.values.insert(
            setting.name,
            Given {
                scopes: setting.scopes,
                value: value.into_value().into_setting_json(),
            },
        );
        self
    }

    /// The value given `name`, as the wire carries it, if any.
    pub(crate) fn given(&self, name: &str) -> Option<&Value> {
        self.values.get(name).map(|given| &given.value)
    }

    /// Sets `setting` to `value` unless a value was given it before, which
    /// stays.
    #[must_use]
    pub(crate) fn or_set<T: SettingValue>(
        self,
        setting: Setting<T>,
        value: impl IntoSettingValue<T>,
    ) -> Self {
        if self.values.contains_key(setting.name) {
            self
        } else {
            self.set(setting, value)
        }
    }

    /// The settings as `scope`'s `settings` member carries them, nested by
    /// the keys' segments: a Run's start's, or a connection's `hello`'s.
    ///
    /// # Errors
    ///
    /// `setting_not_applicable` for the first key, in key order, that the
    /// settings of `scope` do not set, its `fix` naming those that do.
    pub fn within(self, scope: SettingScope) -> Result<Map<String, Value>, Error> {
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
/// [`Duration`](std::time::Duration), a choice's own enum, the protocol's
/// type of a shape it defines once, a `Vec` or a `BTreeMap` of them, and
/// an `Option` of any for a setting that can be cleared: each type whose
/// JSON the protocol's [`SettingJson`] gives, which owns the wire formats.
pub trait SettingValue: SettingJson {}

impl<T: SettingJson> SettingValue for T {}

/// What [`Settings::set`] takes for a key whose values are `T`: a `T`, a
/// `T` for a key that can be cleared, whose values are `Option<T>`, and a
/// `&str` for text. Unlike `Into<T>`, it lets a bare integer literal be a
/// `u64`, since only `u64` itself gives a whole number's key its value.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is no value of a setting whose values are `{T}`",
    label = "the key's values are `{T}`"
)]
pub trait IntoSettingValue<T>: sealed::IntoValue<T> {}

/// What `set` takes into a key's value, which only this crate implements.
mod sealed {
    /// A value `set` takes for a key whose values are `T`.
    pub trait IntoValue<T> {
        /// The key's value.
        fn into_value(self) -> T;
    }
}

impl<T: SettingValue> sealed::IntoValue<T> for T {
    fn into_value(self) -> T {
        self
    }
}

impl<T: SettingValue> IntoSettingValue<T> for T {}

/// A value of a key that can be cleared.
impl<T: SettingValue> sealed::IntoValue<Option<T>> for T {
    fn into_value(self) -> Option<T> {
        Some(self)
    }
}

impl<T: SettingValue> IntoSettingValue<Option<T>> for T {}

impl sealed::IntoValue<String> for &str {
    fn into_value(self) -> String {
        self.to_owned()
    }
}

#[diagnostic::do_not_recommend]
impl IntoSettingValue<String> for &str {}

impl sealed::IntoValue<Option<String>> for &str {
    fn into_value(self) -> Option<String> {
        Some(self.to_owned())
    }
}

#[diagnostic::do_not_recommend]
impl IntoSettingValue<Option<String>> for &str {}

#[cfg(test)]
mod tests;
