//! The run layer's settings on the command line (ADR 0083): `--set
//! <key>=<value>`, repeatable, and `--settings <file|->`, a JSON object
//! from a file or standard input, both the run layer's `flag` origin. A
//! value is read as JSON when it parses as JSON and as text otherwise; a
//! duration setting's value is milliseconds or a suffixed duration (`45s`,
//! `2m`, `1h`), sent as milliseconds (ADR 0080). A bad value is a usage
//! error clap reports while it reads the command line, before any engine
//! call; the engine checks every other value. Only a command whose
//! operation takes `settings` offers the flags; anywhere else clap refuses
//! them as it does any unknown flag.

use std::io::Read as _;

use clap::Args;
use goodfetch_protocol::catalog::{Setting, ValueKind};
use goodfetch_protocol::entries::MODULES;
use serde_json::{Map, Value};

/// The settings flags of a command whose operation takes `settings`.
#[derive(Args, Clone, Debug, Default, PartialEq)]
pub(super) struct SettingsFlags {
    /// Sets a setting for this run, as `key=value`, the value read as JSON
    /// when it parses as JSON and as text otherwise, a duration as
    /// milliseconds or as `45s`, `2m` or `1h`; repeatable
    #[arg(long = "set", value_name = "KEY=VALUE", value_parser = set)]
    set: Vec<Set>,
    /// Reads settings for this run from a JSON object, nested by key, in
    /// FILE, or from standard input for `-`; `--set` beats it
    #[arg(long = "settings", value_name = "FILE|-", value_parser = settings)]
    settings: Option<Settings>,
}

/// One `--set`.
#[derive(Clone, Debug, PartialEq)]
struct Set {
    key: String,
    value: Value,
}

/// What `--settings` read, by canonical dotted key.
#[derive(Clone, Debug, PartialEq)]
struct Settings(Map<String, Value>);

impl SettingsFlags {
    /// The run layer they give, each key in its canonical dotted form:
    /// `--settings`' values, each `--set` over them, a later one over an
    /// earlier one.
    pub(super) fn run_layer(&self) -> Map<String, Value> {
        let mut run_layer = self
            .settings
            .as_ref()
            .map(|settings| settings.0.clone())
            .unwrap_or_default();
        for set in &self.set {
            run_layer.insert(set.key.clone(), set.value.clone());
        }
        run_layer
    }
}

/// Reads `--settings`' JSON object from the file `source` names, or from
/// standard input for `-`.
fn settings(source: &str) -> Result<Settings, String> {
    let text = if source == "-" {
        let mut text = String::new();
        #[expect(
            clippy::disallowed_methods,
            reason = "`--settings -` reads standard input while clap reads the command line, before any engine or transport starts"
        )]
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("standard input does not read: {error}"))?;
        text
    } else {
        #[expect(
            clippy::disallowed_methods,
            reason = "the file is read while clap reads the command line, before any runtime starts"
        )]
        std::fs::read_to_string(source)
            .map_err(|error| format!("`{source}` does not read: {error}"))?
    };
    let Ok(Value::Object(nested)) = serde_json::from_str(&text) else {
        return Err("expected a JSON object of settings, nested by key".to_owned());
    };
    let mut flat = Map::new();
    flatten("", nested, &mut flat)?;
    Ok(Settings(flat))
}

/// Adds the settings `table` holds below `prefix` to `flat`, each by its
/// dotted key: a path is walked until it names a setting. A path that names
/// none is kept as it is, for the engine to refuse.
fn flatten(
    prefix: &str,
    table: Map<String, Value>,
    flat: &mut Map<String, Value>,
) -> Result<(), String> {
    for (name, value) in table {
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}.{name}")
        };
        let is_setting = all_settings().any(|setting| setting.name == path);
        let is_prefix = all_settings().any(|setting| {
            setting
                .name
                .strip_prefix(path.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
        });
        match value {
            Value::Object(table) if !is_setting && is_prefix => flatten(&path, table, flat)?,
            value => {
                let value = as_setting(&path, value)?;
                flat.insert(path, value);
            }
        }
    }
    Ok(())
}

/// Every setting of both Products: a key only the other Product honours
/// reads as its own does, for the engine to refuse (ADR 0087).
fn all_settings() -> impl Iterator<Item = &'static Setting> {
    MODULES.iter().flat_map(|entries| entries.settings)
}

/// Reads `--set`'s `key=value`.
fn set(text: &str) -> Result<Set, String> {
    let (key, value) = text
        .split_once('=')
        .ok_or_else(|| "expected `key=value`".to_owned())?;
    let value = serde_json::from_str(value).unwrap_or_else(|_| Value::from(value));
    Ok(Set {
        value: as_setting(key, value)?,
        key: key.to_owned(),
    })
}

/// `value` as the setting `key` takes it from a command line: a duration
/// setting's value in milliseconds, the wire's one unit (ADR 0080), from
/// milliseconds or a suffixed duration. Any other value, and any other key's
/// value, is the engine's to check.
fn as_setting(key: &str, value: Value) -> Result<Value, String> {
    let duration = all_settings()
        .any(|setting| setting.name == key && matches!(setting.kind, ValueKind::Duration));
    if !duration {
        return Ok(value);
    }
    let milliseconds = match &value {
        Value::Null => return Ok(value),
        Value::Number(number) => number.as_u64(),
        Value::String(text) => milliseconds(text),
        _ => None,
    };
    milliseconds.map(Value::from).ok_or_else(|| {
        format!(
            "`{key}` is a duration: milliseconds, or a whole number with `ms`, `s`, `m` or `h`, such as `45s`, `2m` or `1h`"
        )
    })
}

/// The milliseconds of a duration written as a whole number with no unit
/// or `ms`, `s`, `m` or `h`: the one duration syntax of the command line,
/// which the generated commands' `--timeout` reads too.
pub(crate) fn milliseconds(text: &str) -> Option<u64> {
    let unit_at = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(unit_at);
    let scale: u64 = match unit {
        "" | "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(scale)
}

#[cfg(test)]
mod tests;
