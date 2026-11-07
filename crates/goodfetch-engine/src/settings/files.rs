//! The settings files (ADRs 0028, 0083 and 0098): the Operator policy and
//! user configuration of each major in Goodfetch home, which both Products
//! read, and a project's configuration, found upward from a folder. Each
//! is TOML: a `version` and its tables of settings nested by key. Reading
//! one checks its version under the format registry's rule; rewriting one
//! changes only the value it writes, so every other entry, the other
//! Product's and unknown ones included, keeps its bytes.
//!
//! The Operator policy and user configuration are shared stores (ADR
//! 0098): beside `version` each records `lowest_minor`, the lowest minor
//! of its Product major that can use it, which an engine writing a key its
//! minor added raises ([`set`]), and which every read checks. A project's
//! configuration keeps ADR 0030's default rule, since a project pins its
//! Product version. A new major that finds an Operator policy only for an
//! older major refuses until `upgrade-files` has given it its copy (ADR
//! 0030).
#![expect(
    clippy::disallowed_methods,
    reason = "the files are read on the tasks module's blocking policy, never on an async worker"
)]

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::{Authority, Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{
    Compatibility, Format, OPERATOR_POLICY, Rule, USER_CONFIGURATION,
};
use goodfetch_protocol::settings::{SettingsFile, SettingsLayer};
use serde_json::{Map, Number, Value};
use sha2::{Digest as _, Sha256};
use toml_edit::{DocumentMut, Item};

use super::error::SettingsError;
use super::keys;
use super::resolve::{Layers, Policy, Resolved, resolve};
use super::values::Values;
use crate::home::{Home, major_name, major_order};
use crate::lowest_minor::{Marker, Opener};

/// The key beside `version` that records a shared settings file's lowest
/// minor (ADR 0098).
pub const LOWEST_MINOR_KEY: &str = "lowest_minor";

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
    root: PathBuf,
    version: ProductVersion,
}

impl SettingsFiles {
    /// The files of `version`'s major in `home`, which both Products share
    /// (ADR 0098).
    pub fn of(home: &Home, version: &ProductVersion) -> Self {
        let folder = home.per_major(version);
        Self {
            operator_policy: folder.join(OPERATOR_POLICY_FILE),
            user_configuration: folder.join(USER_CONFIGURATION_FILE),
            root: home.root().to_owned(),
            version: version.clone(),
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

    /// `error`, its `fix` naming these files where it names their layers: a
    /// key only the machine's own layers hold (ADR 0098), set anywhere else,
    /// is to be set in the user configuration or Operator policy file at
    /// these paths.
    #[must_use]
    pub fn named_in(&self, error: SettingsError) -> SettingsError {
        match error {
            SettingsError::NotApplicable { key, layer, .. }
                if keys::find(&key)
                    .is_some_and(|found| matches!(found.setting.authority, Authority::Machine)) =>
            {
                SettingsError::NotApplicable {
                    fix: format!(
                        "set `{key}` in user configuration, `{}`, or an Operator policy, `{}`",
                        self.user_configuration.display(),
                        self.operator_policy.display()
                    ),
                    key,
                    layer,
                }
            }
            error => error,
        }
    }

    /// Reads both, on an engine of `engine` at this major's version; a
    /// missing file sets nothing. Blocking: run it on the tasks module's
    /// blocking policy.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a file that does not read or parse,
    /// `format_version_unsupported` for one of a version this engine cannot
    /// read or whose lowest minor is newer than it, or for an Operator
    /// policy found only for an older major, and the refusals of
    /// [`Values::read`] for its keys.
    pub fn load(&self, engine: Product) -> Result<MachineLayers, SettingsError> {
        let policy_text = read(&self.operator_policy, SettingsLayer::OperatorPolicy)?;
        if policy_text.is_none()
            && let Some(older) = self.older_operator_policy()
        {
            return Err(SettingsError::OlderMajorPolicy {
                path: older.display().to_string(),
                major: older
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                current: major_name(&self.version),
            });
        }
        let user_text = read(&self.user_configuration, SettingsLayer::UserConfiguration)?;
        let operator_policy = match &policy_text {
            Some(text) => operator_policy(text, engine, &self.version)?,
            None => Policy::default(),
        };
        let user_configuration = match &user_text {
            Some(text) => user_configuration(
                SettingsLayer::UserConfiguration,
                text,
                engine,
                &self.version,
            )?,
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

    /// The machine's settings on an engine of `engine`: both files read as
    /// [`Self::load`] reads them and resolved as the engine resolves them
    /// before any Run, on the catalog's defaults. Blocking.
    ///
    /// # Errors
    ///
    /// As [`Self::load`], and as [`MachineLayers::resolve`].
    pub fn machine_settings(&self, engine: Product) -> Result<Resolved, SettingsError> {
        self.load(engine)?.resolve(engine)
    }
}

impl SettingsFiles {
    /// The Operator policy of the newest older major in Goodfetch home, if
    /// any: during `0.y` previews each `0.y` minor counts as a major (ADR
    /// 0030), older than every release major.
    ///
    /// The older majors are looked up by their folders' names, newest first,
    /// so the usual start, with no Operator policy anywhere, never lists
    /// Goodfetch home (a directory stream's 32 KiB buffer at every engine's
    /// start). Only a release major without an older major's policy lists
    /// it, since the previews before it have no known count.
    fn older_operator_policy(&self) -> Option<PathBuf> {
        let (major, minor) = major_order(&major_name(&self.version))?;
        let policy = |folder: String| {
            Some(self.root.join(folder).join(OPERATOR_POLICY_FILE)).filter(|path| path.is_file())
        };
        if major == 0 {
            return (0..minor)
                .rev()
                .find_map(|older| policy(format!("0.{older}")));
        }
        (1..major)
            .rev()
            .find_map(|older| policy(older.to_string()))
            .or_else(|| self.newest_preview_operator_policy())
    }

    /// The Operator policy of the newest `0.y` preview in Goodfetch home, if
    /// any, found by listing it.
    fn newest_preview_operator_policy(&self) -> Option<PathBuf> {
        let entries = std::fs::read_dir(&self.root).ok()?;
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let order @ (0, _) = major_order(&name)? else {
                    return None;
                };
                let policy = entry.path().join(OPERATOR_POLICY_FILE);
                policy.is_file().then_some((order, policy))
            })
            .max_by_key(|(order, _)| *order)
            .map(|(_, policy)| policy)
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

impl MachineLayers {
    /// These layers resolved on an engine of `engine` before any Run, on
    /// the catalog's defaults.
    ///
    /// # Errors
    ///
    /// The refusals of [`resolve`].
    pub fn resolve(self, engine: Product) -> Result<Resolved, SettingsError> {
        resolve(
            &Layers {
                operator_policy: self.operator_policy,
                user_configuration: self.user_configuration,
                ..Layers::default()
            },
            engine,
            engine,
        )
    }
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

/// Reads user configuration's text, on an engine of `engine` at the Product
/// version `version`, whose minor must meet the file's lowest minor (ADR
/// 0098).
///
/// # Errors
///
/// As [`configuration`], and `format_version_unsupported` for a file whose
/// lowest minor is newer than `version`.
pub fn user_configuration(
    layer: SettingsLayer,
    text: &str,
    engine: Product,
    version: &ProductVersion,
) -> Result<Values, SettingsError> {
    let tables = file_tables(
        layer,
        text,
        &USER_CONFIGURATION,
        &["settings"],
        Some(version),
    )?;
    Values::read(layer, &tables[0], engine)
}

/// The settings of the project configuration at `path`, nested as the
/// file nests them, as a Client sends them to a Daemon's `config.explain`;
/// `None` when there is no such file. Blocking.
///
/// # Errors
///
/// `setting_invalid` for a file that is no project configuration.
pub fn project_settings(path: &Path) -> Result<Option<Map<String, Value>>, SettingsError> {
    let layer = SettingsLayer::ProjectConfiguration;
    match read(path, layer)? {
        Some(text) => Ok(
            file_tables(layer, &text, &USER_CONFIGURATION, &["settings"], None)?
                .into_iter()
                .next(),
        ),
        None => Ok(None),
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

/// Reads a project configuration's text, which keeps ADR 0030's default
/// rule.
///
/// # Errors
///
/// `setting_invalid` for text that is no such file,
/// `format_version_unsupported` for one of a version this engine cannot
/// read, and the refusals of [`Values::read`] for its keys.
pub fn configuration(
    layer: SettingsLayer,
    text: &str,
    engine: Product,
) -> Result<Values, SettingsError> {
    let tables = file_tables(layer, text, &USER_CONFIGURATION, &["settings"], None)?;
    Values::read(layer, &tables[0], engine)
}

/// Reads an Operator policy's text, whose version is its digest, on an
/// engine of `engine` at the Product version `version`.
///
/// # Errors
///
/// As [`user_configuration`], and `setting_invalid` for a cap on a key that
/// is no count or duration.
pub fn operator_policy(
    text: &str,
    engine: Product,
    version: &ProductVersion,
) -> Result<Policy, SettingsError> {
    let layer = SettingsLayer::OperatorPolicy;
    let tables = file_tables(
        layer,
        text,
        &OPERATOR_POLICY,
        &["settings", "lock", "cap", "managed"],
        Some(version),
    )?;
    let marked_managed = text
        .parse::<DocumentMut>()
        .is_ok_and(|document| document.contains_key("managed"));
    let managed = if marked_managed {
        Some(
            serde_json::from_value(Value::Object(tables[3].clone())).map_err(|error| {
                invalid_file(
                    OPERATOR_POLICY.name.to_owned(),
                    layer,
                    format!("`managed` does not read: {error}"),
                )
            })?,
        )
    } else {
        None
    };
    let policy = Policy {
        settings: Values::read(layer, &tables[0], engine)?,
        lock: Values::read(layer, &tables[1], engine)?,
        cap: Values::read(layer, &tables[2], engine)?,
        version: Some(policy_version(text)),
        managed,
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
/// `version` and refusing any other top-level entry: a shared file, read by
/// an engine at the Product version `shared`, under `format`'s rule and its
/// lowest minor (ADR 0098), and a project's under ADR 0030's default rule.
fn file_tables(
    layer: SettingsLayer,
    text: &str,
    format: &'static Format,
    tables: &[&str],
    shared: Option<&ProductVersion>,
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
    let lowest_minor = match top.remove(LOWEST_MINOR_KEY) {
        None => 0,
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|number| u16::try_from(number).ok())
            .ok_or_else(|| invalid(format!("`{LOWEST_MINOR_KEY}` is no minor")))?,
        Some(_) => return Err(invalid(format!("`{LOWEST_MINOR_KEY}` is no minor"))),
    };
    let refusal = match shared {
        Some(version) => Opener::new(format, version.major_minor())
            .check(&Marker {
                version: found,
                lowest_minor,
            })
            .err()
            .map(|refusal| (refusal.compatibility, refusal.install)),
        None => match Rule::Default.compatibility(format.version, found) {
            Compatibility::Compatible => None,
            other => Some((other, None)),
        },
    };
    if let Some((compatibility, install)) = refusal {
        return Err(SettingsError::Format {
            layer,
            format: format.name,
            found: found.to_string(),
            supported: format.version.to_string(),
            compatibility,
            install,
        });
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

/// `text`, a shared settings file (user configuration or an Operator
/// policy), with `key` set to `value` in its `table` as [`rewrite`] sets
/// it, written by an engine of `format` at the Product version `version`:
/// it records its format version, and raises the file's lowest minor to
/// the minor that added `key`, `added_in` (the Product major and minor the
/// catalog records), when that is its own major's (ADR 0098).
///
/// # Errors
///
/// As [`rewrite`], and `setting_invalid` for a `version` or
/// `lowest_minor` that does not read.
pub fn set(
    text: &str,
    table: &str,
    key: &str,
    value: &Value,
    format: &'static Format,
    version: &ProductVersion,
    added_in: ProtocolVersion,
) -> Result<String, SettingsError> {
    let rewritten = rewrite(text, table, key, value)?;
    let invalid = |reason: String| SettingsError::Invalid {
        key: format.name.to_owned(),
        layer: None,
        reason,
    };
    let mut document: DocumentMut = rewritten
        .parse()
        .map_err(|error: toml_edit::TomlError| invalid(error.message().to_owned()))?;
    let opener = Opener::new(format, version.major_minor());
    let found = Marker {
        version: match document.get("version").and_then(Item::as_str) {
            Some(version) => version
                .parse()
                .map_err(|_| invalid(format!("`{version}` is no `major.minor` version")))?,
            None => format.version,
        },
        lowest_minor: match document.get(LOWEST_MINOR_KEY) {
            None => 0,
            Some(item) => item
                .as_integer()
                .and_then(|minor| u16::try_from(minor).ok())
                .ok_or_else(|| invalid(format!("`{LOWEST_MINOR_KEY}` is no minor")))?,
        },
    };
    let marked = opener.written(found, Some(added_in));
    if marked.version != found.version {
        set_top(
            &mut document,
            "version",
            toml_edit::Value::from(marked.version.to_string()),
        );
    }
    if marked.lowest_minor != found.lowest_minor {
        set_top(
            &mut document,
            LOWEST_MINOR_KEY,
            toml_edit::Value::from(i64::from(marked.lowest_minor)),
        );
    }
    Ok(document.to_string())
}

/// Sets a top-level `key` of `document` to `value`, keeping its decor when
/// it has one.
fn set_top(document: &mut DocumentMut, key: &str, value: toml_edit::Value) {
    match document.get_mut(key).and_then(Item::as_value_mut) {
        Some(existing) => {
            let decor = existing.decor().clone();
            *existing = value;
            *existing.decor_mut() = decor;
        }
        None => {
            document.insert(key, Item::Value(value));
        }
    }
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
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version");
    for product in [Product::Collie, Product::Foxhound] {
        let _ = operator_policy(text, product, &version);
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
