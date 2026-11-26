//! The Plugin lockfile's text (ADR 0033), `goodfetch-plugins.lock`: the
//! engine reads its TOML as JSON, holds its version to the
//! `plugin-lockfile` format's rule (ADR 0066) before the rest, and writes it
//! back as `version`, then one `[[plugin]]` table for each Plugin it pins.

use goodfetch_protocol::formats::{Compatibility, PLUGIN_LOCKFILE};
use goodfetch_protocol::plugin::add::PLUGIN_LOCKFILE_INVALID;
use goodfetch_protocol::plugin::lockfile::PluginLockfile;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::call_error::CallError;
use crate::run::format_refusal;
use crate::settings::json_of_table;

/// Reads a lockfile's text.
///
/// # Errors
///
/// `plugin_lockfile_invalid` for text that is no lockfile, and
/// `format_version_unsupported` for one of a version this engine does not
/// read.
pub fn read(text: &str) -> Result<PluginLockfile, CallError> {
    let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
        invalid(format!("it is not TOML: {}", error.message()))
    })?;
    let document = json_of_table(document.as_table());
    let found =
        PluginLockfile::version_of(&document).map_err(|error| invalid(error.to_string()))?;
    match PLUGIN_LOCKFILE
        .rule
        .compatibility(PLUGIN_LOCKFILE.version, found)
    {
        Compatibility::Compatible => {
            PluginLockfile::read(document).map_err(|error| invalid(error.to_string()))
        }
        refused => Err(format_refusal(
            &PLUGIN_LOCKFILE,
            &found.to_string(),
            refused,
        )),
    }
}

/// A lockfile's text, as the engine writes it.
#[must_use]
pub fn text(lockfile: &PluginLockfile) -> String {
    let mut document = DocumentMut::new();
    document["version"] = value(lockfile.version.to_string());
    let mut plugins = ArrayOfTables::new();
    for plugin in &lockfile.plugins {
        let mut table = Table::new();
        table["name"] = value(&plugin.name);
        table["version"] = value(&plugin.version);
        table["digest"] = value(&plugin.digest);
        if let Some(signer) = &plugin.signer {
            table["signer"] = value(signer);
        }
        if let Some(approvals) = &plugin.approvals {
            table["approvals"] = value(approvals);
        }
        plugins.push(table);
    }
    if !plugins.is_empty() {
        document["plugin"] = Item::ArrayOfTables(plugins);
    }
    document.to_string()
}

/// `plugin_lockfile_invalid`, its message naming `reason`.
fn invalid(reason: impl std::fmt::Display) -> CallError {
    CallError::new(
        &PLUGIN_LOCKFILE_INVALID,
        format!("The Plugin lockfile is refused: {reason}."),
        &(),
    )
}

#[cfg(test)]
mod tests;
