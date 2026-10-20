//! The settings files (ADRs 0028, 0083 and 0098): the Operator policy and
//! user configuration of each major in Goodfetch home, which both Products
//! read, and a project's configuration, found upward from a folder. Each
//! is TOML: a `version` and its tables of settings nested by key. Reading
//! one checks its version under the format registry's rule; rewriting one
//! changes only the value it writes, so every other entry, the other
//! Product's and unknown ones included, keeps its bytes.
#![expect(
    clippy::disallowed_methods,
    reason = "the files are read on the tasks module's blocking policy, never on an async worker"
)]

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{Compatibility, Format, OPERATOR_POLICY, USER_CONFIGURATION};
use goodfetch_protocol::settings::{SettingsFile, SettingsLayer};
use serde_json::{Map, Number, Value};
use sha2::{Digest as _, Sha256};
use toml_edit::{DocumentMut, Item};

use super::error::SettingsError;
use super::resolve::Policy;
use super::values::Values;
use crate::home::Home;

/// The Operator policy's file in the per-major folder.
pub const OPERATOR_POLICY_FILE: &str = "operator-policy.toml";

/// User configuration's file in the per-major folder.
pub const USER_CONFIGURATION_FILE: &str = "configuration.toml";

/// A project configuration's file, found in a folder or above it.
pub const PROJECT_CONFIGURATION_FILE: &str = "goodfetch.toml";

/// Where one major's Operator policy and user configuration are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsFiles {
    operator_policy: PathBuf,
    user_configuration: PathBuf,
}

impl SettingsFiles {
    /// The files of `version`'s major in `home`, which both Products share
    /// (ADR 0098).
    pub fn of(home: &Home, version: &ProductVersion) -> Self {
        let folder = home.per_major(version);
        Self {
            operator_policy: folder.join(OPERATOR_POLICY_FILE),
            user_configuration: folder.join(USER_CONFIGURATION_FILE),
        }
    }

    /// The Operator policy's path.
    pub fn operator_policy(&self) -> &Path {
        &self.operator_policy
    }

    /// User configuration's path.
    pub fn user_configuration(&self) -> &Path {
        &self.user_configuration
    }

    /// Reads both, on an engine of `engine`; a missing file sets nothing.
    /// Blocking: run it on the tasks module's blocking policy.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a file that does not read or parse, or of a
    /// version this engine cannot read, and the refusals of
    /// [`Values::read`] for its keys.
    pub fn load(&self, engine: Product) -> Result<MachineLayers, SettingsError> {
        let policy_text = read(&self.operator_policy, SettingsLayer::OperatorPolicy)?;
        let user_text = read(&self.user_configuration, SettingsLayer::UserConfiguration)?;
        let operator_policy = match &policy_text {
            Some(text) => operator_policy(text, engine)?,
            None => Policy::default(),
        };
        let user_configuration = match &user_text {
            Some(text) => configuration(SettingsLayer::UserConfiguration, text, engine)?,
            None => Values::default(),
        };
        Ok(MachineLayers {
            operator_policy,
            user_configuration,
            files: vec![
                SettingsFile {
                    layer: SettingsLayer::OperatorPolicy,
                    path: self.operator_policy.display().to_string(),
                    found: policy_text.is_some(),
                },
                SettingsFile {
                    layer: SettingsLayer::UserConfiguration,
                    path: self.user_configuration.display().to_string(),
                    found: user_text.is_some(),
                },
            ],
        })
    }
}

/// The layers a machine keeps: its Operator policy and user configuration,
/// and the files they came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineLayers {
    /// The Operator policy.
    pub operator_policy: Policy,
    /// User configuration.
    pub user_configuration: Values,
    /// The files read.
    pub files: Vec<SettingsFile>,
}

/// The project configuration file in `folder` or the nearest folder above
/// it (ADR 0083). Blocking.
pub fn find_project(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .map(|folder| folder.join(PROJECT_CONFIGURATION_FILE))
        .find(|path| path.is_file())
}

/// Reads the project configuration at `path`, on an engine of `engine`.
/// Blocking.
///
/// # Errors
///
/// As [`SettingsFiles::load`].
pub fn load_project(path: &Path, engine: Product) -> Result<Values, SettingsError> {
    match read(path, SettingsLayer::ProjectConfiguration)? {
        Some(text) => configuration(SettingsLayer::ProjectConfiguration, &text, engine),
        None => Ok(Values::default()),
    }
}

fn read(path: &Path, layer: SettingsLayer) -> Result<Option<String>, SettingsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(invalid_file(
            path.display().to_string(),
            layer,
            error.to_string(),
        )),
    }
}

fn invalid_file(file: String, layer: SettingsLayer, reason: String) -> SettingsError {
    SettingsError::Invalid {
        key: file,
        layer: Some(layer),
        reason,
    }
}

/// Reads a user or project configuration's text.
///
/// # Errors
///
/// `setting_invalid` for text that is no such file, and the refusals of
/// [`Values::read`] for its keys.
pub fn configuration(
    layer: SettingsLayer,
    text: &str,
    engine: Product,
) -> Result<Values, SettingsError> {
    let tables = file_tables(layer, text, &USER_CONFIGURATION, &["settings"])?;
    Values::read(layer, &tables[0], engine)
}

/// Reads an Operator policy's text, whose version is its digest.
///
/// # Errors
///
/// As [`configuration`], and `setting_invalid` for a cap on a key that is
/// no count or duration.
pub fn operator_policy(text: &str, engine: Product) -> Result<Policy, SettingsError> {
    let layer = SettingsLayer::OperatorPolicy;
    let tables = file_tables(layer, text, &OPERATOR_POLICY, &["settings", "lock", "cap"])?;
    let policy = Policy {
        settings: Values::read(layer, &tables[0], engine)?,
        lock: Values::read(layer, &tables[1], engine)?,
        cap: Values::read(layer, &tables[2], engine)?,
        version: Some(policy_version(text)),
    };
    policy.check_caps()?;
    Ok(policy)
}

/// An Operator policy's version: `sha256:` and the first 16 hex digits of
/// its text's digest.
pub fn policy_version(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest
        .iter()
        .take(8)
        .fold("sha256:".to_owned(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// The file's `tables`, each empty when absent, after checking its
/// `version` under `format`'s rule and refusing any other top-level entry.
fn file_tables(
    layer: SettingsLayer,
    text: &str,
    format: &Format,
    tables: &[&str],
) -> Result<Vec<Map<String, Value>>, SettingsError> {
    let invalid = |reason: String| invalid_file(format.name.to_owned(), layer, reason);
    let document: DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| invalid(error.message().to_owned()))?;
    let Value::Object(mut top) = json_of_table(document.as_table()) else {
        return Err(invalid("expected a table".to_owned()));
    };
    let version = top
        .remove("version")
        .and_then(|version| version.as_str().map(str::to_owned))
        .ok_or_else(|| invalid("it names no `version`".to_owned()))?;
    let found: ProtocolVersion = version
        .parse()
        .map_err(|_| invalid(format!("`{version}` is no `major.minor` version")))?;
    match format.rule.compatibility(format.version, found) {
        Compatibility::Compatible => {}
        Compatibility::NewerMinor | Compatibility::NewerMajor => {
            return Err(invalid(format!(
                "it was written in {} {found}, newer than this engine's {}; upgrade the engine",
                format.name, format.version
            )));
        }
        Compatibility::OlderMajor => {
            return Err(invalid(format!(
                "it was written in {} {found}, an older major; run `upgrade-files`",
                format.name
            )));
        }
    }
    let mut found_tables = Vec::new();
    for table in tables {
        match top.remove(*table) {
            None => found_tables.push(Map::new()),
            Some(Value::Object(entries)) => found_tables.push(entries),
            Some(_) => return Err(invalid(format!("`{table}` is not a table"))),
        }
    }
    if let Some(other) = top.keys().next() {
        return Err(invalid(format!(
            "`{other}` is no part of this file; settings go in `[settings]`"
        )));
    }
    Ok(found_tables)
}

fn json_of_table(table: &toml_edit::Table) -> Value {
    Value::Object(
        table
            .iter()
            .filter_map(|(name, item)| json_of_item(item).map(|value| (name.to_owned(), value)))
            .collect(),
    )
}

fn json_of_item(item: &Item) -> Option<Value> {
    match item {
        Item::None => None,
        Item::Value(value) => Some(json_of_value(value)),
        Item::Table(table) => Some(json_of_table(table)),
        Item::ArrayOfTables(tables) => {
            Some(Value::Array(tables.iter().map(json_of_table).collect()))
        }
    }
}

fn json_of_value(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(text) => Value::String(text.value().clone()),
        toml_edit::Value::Integer(number) => Value::Number(Number::from(*number.value())),
        toml_edit::Value::Float(number) => {
            Number::from_f64(*number.value()).map_or(Value::Null, Value::Number)
        }
        toml_edit::Value::Boolean(boolean) => Value::Bool(*boolean.value()),
        toml_edit::Value::Datetime(datetime) => Value::String(datetime.value().to_string()),
        toml_edit::Value::Array(array) => Value::Array(array.iter().map(json_of_value).collect()),
        toml_edit::Value::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(name, value)| (name.to_owned(), json_of_value(value)))
                .collect(),
        ),
    }
}

/// `text`, a settings file, with `key` set to `value` in its `table`
/// (`settings`, or an Operator policy's `lock` or `cap`), every other byte
/// kept: the other Product's entries, unknown ones and comments too
/// (ADRs 0087 and 0098). `null` removes the key.
///
/// # Errors
///
/// `setting_invalid` for text that does not parse, or a value TOML cannot
/// hold.
pub fn rewrite(text: &str, table: &str, key: &str, value: &Value) -> Result<String, SettingsError> {
    let invalid = |reason: String| SettingsError::Invalid {
        key: key.to_owned(),
        layer: None,
        reason,
    };
    let mut document: DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| invalid(error.message().to_owned()))?;
    let mut segments: Vec<&str> = std::iter::once(table).chain(key.split('.')).collect();
    let last = segments
        .pop()
        .ok_or_else(|| invalid("an empty key".to_owned()))?;
    let mut current = document.as_table_mut();
    for segment in segments {
        let entry = current.entry(segment).or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            Item::Table(table)
        });
        current = entry
            .as_table_mut()
            .ok_or_else(|| invalid(format!("`{segment}` is not a table")))?;
    }
    if value.is_null() {
        current.remove(last);
    } else {
        let written = toml_of(value).ok_or_else(|| invalid("TOML holds no `null`".to_owned()))?;
        match current.get_mut(last).and_then(Item::as_value_mut) {
            Some(existing) => {
                let decor = existing.decor().clone();
                *existing = written;
                *existing.decor_mut() = decor;
            }
            None => {
                current.insert(last, Item::Value(written));
            }
        }
    }
    Ok(document.to_string())
}

fn toml_of(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(boolean) => toml_edit::Value::from(*boolean),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => toml_edit::Value::from(integer),
            None => toml_edit::Value::from(number.as_f64()?),
        },
        Value::String(text) => toml_edit::Value::from(text.as_str()),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(toml_of(item)?);
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(entries) => {
            let mut table = toml_edit::InlineTable::new();
            for (name, entry) in entries {
                table.insert(name, toml_of(entry)?);
            }
            toml_edit::Value::InlineTable(table)
        }
    })
}

/// The settings files' fuzz target's body (ADR 0067): reads `data` as an
/// Operator policy and as user configuration, on both Products' engines;
/// a configuration that reads, rewritten with each value it applies, reads
/// back the same. It panics only on a bug.
pub fn fuzz_settings_file(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for product in [Product::Collie, Product::Foxhound] {
        let _ = operator_policy(text, product);
        let Ok(read) = configuration(SettingsLayer::UserConfiguration, text, product) else {
            continue;
        };
        for (key, value) in &read.applied {
            let rewritten = rewrite(text, "settings", key, value)
                .expect("invariant: a file that read rewrites");
            let again = configuration(SettingsLayer::UserConfiguration, &rewritten, product)
                .expect("invariant: a rewritten file reads");
            assert_eq!(
                again.applied, read.applied,
                "a rewrite changed what the file sets"
            );
        }
    }
}
