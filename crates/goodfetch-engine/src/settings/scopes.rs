//! What a scope's own `settings` take, and inherit from the scope opened
//! above it (ADR 0083): only the keys `setting_scopes` gives the scope, a
//! key another scope's code sets refused with `setting_not_applicable`
//! naming those scopes, over the Connection's own settings key by key, so
//! what a Run leaves out it inherits, its `null` clears and its value sets.
//! The Connection's own `settings` also take a Client's transport bounds
//! only from a Client that applies them (issue 2518).

use goodfetch_protocol::catalog::{Product, Setting, SettingScope};
use goodfetch_protocol::connection::{
    CONNECTION_MAX_LISTENER_QUEUE, CONNECTION_MAX_PENDING_CALLS, CONNECTION_MAX_QUEUED_WRITE_BYTES,
    Surface, applies_client_bound,
};
use goodfetch_protocol::setting_scopes;
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::{Map, Value};

use super::error::SettingsError;
use super::setting;
use super::values::Values;

/// The settings a Client applies to its own transport, of
/// `Authority::ConnectionAlone`, which the engine takes only from a Client
/// that applies them.
const CLIENT_BOUNDS: [&Setting; 3] = [
    &CONNECTION_MAX_PENDING_CALLS,
    &CONNECTION_MAX_QUEUED_WRITE_BYTES,
    &CONNECTION_MAX_LISTENER_QUEUE,
];

/// Refuses `own`, a scope's own `settings` as `engine` reads them, when
/// one sets a key that a program's code sets only at other scopes, as
/// `timeout.run`, which only the Connection takes. A key no program's code
/// sets at any scope, as `proxy.providers`, is left to
/// [`Settings::open`](super::Settings::open), which refuses it naming the
/// layer and the file that set it.
///
/// # Errors
///
/// `setting_not_applicable` for the first such key, its `fix` naming the
/// scopes that take it; as [`Values::read`] for values it cannot read.
pub fn check_scope(
    own: &Map<String, Value>,
    scope: SettingScope,
    engine: Product,
) -> Result<(), SettingsError> {
    let own = Values::read(SettingsLayer::Run, own, engine)?;
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

/// Refuses `own`, a `hello`'s settings from `surface` as `engine` reads
/// them, when one sets a bound the Client of `surface` does not apply to
/// its transport, where the value could not take effect (issue 2518).
///
/// # Errors
///
/// `setting_not_applicable` for the first such key; as [`Values::read`]
/// for values it cannot read.
pub fn check_client_bounds(
    own: &Map<String, Value>,
    surface: Surface,
    engine: Product,
) -> Result<(), SettingsError> {
    let unapplied: Vec<&Setting> = CLIENT_BOUNDS
        .into_iter()
        .filter(|bound| !applies_client_bound(surface, bound))
        .collect();
    if unapplied.is_empty() {
        return Ok(());
    }
    let own = Values::read(SettingsLayer::Run, own, engine)?;
    let not_applied_by_the_client = own
        .keys()
        .find(|key| unapplied.iter().any(|bound| bound.name == *key));
    match not_applied_by_the_client {
        Some(key) => Err(SettingsError::NotApplicable {
            key: key.to_owned(),
            layer: SettingsLayer::Run,
            fix: format!(
                "remove `{key}`: this Client does not read it from `settings`; it takes it, where it has it, as an option of its own `connect`"
            ),
        }),
        None => Ok(()),
    }
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
