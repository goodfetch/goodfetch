//! A program's own settings at each scope it opens (ADR 0083): a scope's
//! `settings` take only the keys `setting_scopes` gives it, a key another
//! scope's code sets refused with `setting_not_applicable` naming those
//! scopes; and a Run's run layer takes its Connection's own settings
//! beneath the Run's, key by key, so what the Run leaves out it inherits,
//! its `null` clears and its value sets.

use goodfetch_protocol::catalog::{Product, SettingScope};
use goodfetch_protocol::setting_scopes;
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::{Map, Value};

use super::error::SettingsError;
use super::setting;
use super::values::Values;

/// Refuses `own`, the values a scope's own `settings` give, when one sets
/// a key that a program's code sets only at other scopes, as `timeout.run`,
/// which only the Connection takes. A key no program's code sets at any
/// scope, as `proxy.providers`, is left to [`Settings::open`](super::Settings::open),
/// which refuses it naming the layer and the file that set it.
///
/// # Errors
///
/// `setting_not_applicable` for the first such key, its `fix` naming the
/// scopes that take it.
pub fn check_scope(own: &Values, scope: SettingScope) -> Result<(), SettingsError> {
    for key in own.keys() {
        let Some(setting) = setting(key) else {
            continue;
        };
        let scopes = setting_scopes::of(setting);
        if scopes.is_empty() || scopes.contains(&scope) {
            continue;
        }
        return Err(SettingsError::NotApplicable {
            key: key.to_owned(),
            layer: SettingsLayer::Run,
            fix: set_in(scopes),
        });
    }
    Ok(())
}

/// The fix naming `scopes`, the ones whose own `settings` take a key.
fn set_in(scopes: &[SettingScope]) -> String {
    let names: Vec<&str> = scopes.iter().map(|scope| opener(*scope)).collect();
    format!(
        "set it in the `settings` of {}, which every scope opened beneath inherits",
        names.join(" or ")
    )
}

/// What opens `scope` in a program.
fn opener(scope: SettingScope) -> &'static str {
    match scope {
        SettingScope::Connection => "`connect()`",
        SettingScope::Run => "a Run's start",
        SettingScope::Label => "a crawl's Label",
        SettingScope::Session => "a Session's opening",
        SettingScope::Page => "a page's opening",
    }
}

/// The run layer of a Run started on a Connection: the Connection's own
/// settings `connection`, beneath the Run's own `own`, key by key, each
/// key written whole as one dotted key, so a value that is a table
/// replaces the Connection's whole. `own` as given when the Connection
/// set none.
///
/// # Errors
///
/// As [`Values::read`], for either.
pub fn beneath_connection(
    connection: &Map<String, Value>,
    own: &Map<String, Value>,
    engine: Product,
) -> Result<Map<String, Value>, SettingsError> {
    if connection.is_empty() {
        return Ok(own.clone());
    }
    let connection = Values::read(SettingsLayer::Run, connection, engine)?;
    let own = Values::read(SettingsLayer::Run, own, engine)?;
    Ok(connection
        .applied
        .into_iter()
        .chain(own.applied)
        .map(|(key, value)| (key.to_owned(), value))
        .collect())
}
