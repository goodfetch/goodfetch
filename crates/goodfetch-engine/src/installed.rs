//! The list of installed executables (ADR 0098): one per-user file in
//! Goodfetch home, outside the per-major folders, in which each Product's
//! executable, and each program linking the engine through a Product's Rust
//! crate, records itself when it runs. It answers which executable runs
//! each major's Daemon, its pick, and which Pinned artifacts the Browser
//! cache must keep. The list is of the Usage ledger's family: a rewrite
//! keeps every entry and field this engine does not know, and a list of
//! another major is read as empty and never rewritten.
//!
//! Its file is read and written on the tasks module's blocking policy, or
//! by an executable before its runtime starts, under a lock beside it.
#![expect(
    clippy::disallowed_methods,
    reason = "the list's file is read and written on the tasks module's blocking policy or before any runtime starts, never on an async worker"
)]

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable};
use goodfetch_protocol::formats::{Compatibility, INSTALLED_EXECUTABLES};
use serde_json::{Map, Value};

use crate::browser_cache::{self, MaxUnusedAge};
use crate::durable;
use crate::home::Home;
use crate::run_index;
use crate::settings::{LOWEST_MINOR_KEY, SettingsFiles};

/// The list's file in Goodfetch home.
const FILE: &str = goodfetch_protocol::home::INSTALLED_EXECUTABLES;

/// The lock that every rewrite of the list holds.
const LOCK: &str = "installed-executables.lock";

/// The list of installed executables of one Goodfetch home.
#[derive(Clone, Debug)]
pub struct InstalledList {
    path: PathBuf,
    lock: PathBuf,
}

impl InstalledList {
    /// The list of `home`.
    pub fn of(home: &Home) -> Self {
        Self {
            path: home.root().join(FILE),
            lock: home.root().join(LOCK),
        }
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Records `executable`, replacing the entry of the same kind, path and
    /// Product version, and keeping every other entry and every field this
    /// engine does not know. An entry whose file changed loses its
    /// versioned copy, which is of the earlier file, unless `executable`
    /// names one.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or written, or it is of another major,
    /// which this engine never rewrites.
    pub fn record(&self, executable: &InstalledExecutable) -> io::Result<()> {
        self.rewrite(|entries| {
            let mut recorded = serde_json::to_value(executable)
                .expect("invariant: an entry serializes")
                .as_object()
                .cloned()
                .expect("invariant: an entry is an object");
            let found = entries
                .iter_mut()
                .find(|stored| same_executable(stored, executable));
            match found {
                Some(Value::Object(stored)) => {
                    let same_file = stored.get("size") == recorded.get("size")
                        && stored.get("modified_at") == recorded.get("modified_at");
                    if executable.copy.is_none() {
                        match stored.get("copy") {
                            Some(copy) if same_file => {
                                recorded.insert("copy".to_owned(), copy.clone());
                            }
                            _ => {
                                stored.remove("copy");
                            }
                        }
                    }
                    stored.extend(recorded);
                }
                _ => entries.push(Value::Object(recorded)),
            }
        })
    }

    /// Forgets the entry of `executable`'s kind, path and Product version,
    /// keeping every other entry and every field this engine does not know;
    /// nothing when there is none.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or written, or it is of another major,
    /// which this engine never rewrites.
    pub fn forget(&self, executable: &InstalledExecutable) -> io::Result<()> {
        self.rewrite(|entries| entries.retain(|stored| !same_executable(stored, executable)))
    }

    /// Rewrites the list's entries with `change` under the list's lock.
    fn rewrite(&self, change: impl FnOnce(&mut Vec<Value>)) -> io::Result<()> {
        fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.lock)?;
        lock.lock()?;
        let mut document = match self.document()? {
            Some(Document::Readable(document)) => document,
            Some(Document::OtherMajor) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} is of another major than {}, and is never rewritten",
                        self.path.display(),
                        INSTALLED_EXECUTABLES.version
                    ),
                ));
            }
            None => Map::from_iter([(
                "format_version".to_owned(),
                Value::String(INSTALLED_EXECUTABLES.version.to_string()),
            )]),
        };
        let entries = document
            .entry("executables")
            .or_insert_with(|| Value::Array(Vec::new()));
        if !entries.is_array() {
            *entries = Value::Array(Vec::new());
        }
        change(
            entries
                .as_array_mut()
                .expect("invariant: `executables` was just made an array"),
        );
        let text = serde_json::to_vec_pretty(&Value::Object(document))
            .expect("invariant: the list serializes");
        durable::replace(&self.path, &text)?;
        drop(lock);
        Ok(())
    }

    /// Every entry this engine can read; none from a list of another major
    /// or a missing one.
    ///
    /// # Errors
    ///
    /// When the file exists but cannot be read.
    pub fn read(&self) -> io::Result<Vec<InstalledExecutable>> {
        Ok(match self.document()? {
            Some(Document::Readable(mut document)) => match document.remove("executables") {
                Some(Value::Array(entries)) => entries
                    .into_iter()
                    .filter_map(|entry| serde_json::from_value(entry).ok())
                    .collect(),
                _ => Vec::new(),
            },
            Some(Document::OtherMajor) | None => Vec::new(),
        })
    }

    fn document(&self) -> io::Result<Option<Document>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let Ok(Value::Object(document)) = serde_json::from_slice::<Value>(&bytes) else {
            // A list no engine can read is no list: it is replaced.
            return Ok(None);
        };
        let version = document
            .get("format_version")
            .and_then(Value::as_str)
            .and_then(|version| version.parse::<ProtocolVersion>().ok());
        let Some(version) = version else {
            return Ok(None);
        };
        Ok(Some(
            match INSTALLED_EXECUTABLES
                .rule
                .compatibility(INSTALLED_EXECUTABLES.version, version)
            {
                Compatibility::Compatible => Document::Readable(document),
                Compatibility::NewerMinor
                | Compatibility::OlderMajor
                | Compatibility::NewerMajor => Document::OtherMajor,
            },
        ))
    }
}

enum Document {
    Readable(Map<String, Value>),
    OtherMajor,
}

/// Whether `stored` is the entry of `executable`: the same kind, path and
/// Product version.
fn same_executable(stored: &Value, executable: &InstalledExecutable) -> bool {
    let recorded = serde_json::to_value(executable).expect("invariant: an entry serializes");
    ["kind", "path", "product_version"]
        .iter()
        .all(|field| stored.get(field) == recorded.get(field))
}

/// The size and the modification time of the file at `path`, which tell
/// whether it is still the executable an entry recorded.
///
/// # Errors
///
/// When the file's metadata cannot be read.
pub fn fingerprint(path: &Path) -> io::Result<(u64, SystemTime)> {
    let metadata = fs::metadata(path)?;
    let modified = jiff::Timestamp::try_from(metadata.modified()?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // Whole seconds, so a file system with coarser times reads the same.
    let modified = jiff::Timestamp::from_second(modified.as_second())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((metadata.len(), SystemTime::from(modified)))
}

/// Whether the file at the entry's path is still the executable it
/// recorded, at its size and modification time.
pub fn still_at_path(entry: &InstalledExecutable) -> bool {
    fingerprint(Path::new(&entry.path)).is_ok_and(|found| found == (entry.size, entry.modified_at))
}

/// Whether an entry counts as installed (ADR 0098): its executable is still
/// at its path at its version, or it was seen within `max_unused_age`.
pub fn counts_as_installed(
    still_at_path: bool,
    last_seen: SystemTime,
    now: jiff::Timestamp,
    max_unused_age: MaxUnusedAge,
) -> bool {
    if still_at_path {
        return true;
    }
    match SystemTime::from(now).duration_since(last_seen) {
        Ok(elapsed) => elapsed < max_unused_age.counting(),
        Err(_) => true,
    }
}

/// One entry of the list as the machine shows it now.
#[derive(Clone, Copy, Debug)]
pub struct Seen<'a> {
    /// The entry.
    pub entry: &'a InstalledExecutable,
    /// Whether its executable is still at its path, at its version.
    pub still_at_path: bool,
    /// Whether its versioned copy is in the Browser cache.
    pub copy_present: bool,
}

impl<'a> Seen<'a> {
    /// `entry`, looked at on the machine: at its path, and its copy in the
    /// Browser cache at `cache`.
    pub fn look(entry: &'a InstalledExecutable, cache: &Path) -> Self {
        Self {
            entry,
            still_at_path: still_at_path(entry),
            copy_present: entry
                .copy
                .as_ref()
                .is_some_and(|copy| browser_cache::entry_present(cache, copy)),
        }
    }
}

/// The pick of the major of `version` (ADR 0098): among the Products'
/// executables of that major that count as installed, can be started (at
/// their path, or from their versioned copy) and are of at least the
/// shared stores' lowest minor, Foxhound before Collie, then the newest
/// version. `None` when no executable can be the pick.
pub fn pick<'a>(
    seen: &[Seen<'a>],
    version: &ProductVersion,
    lowest_minor: Option<ProtocolVersion>,
    now: jiff::Timestamp,
    max_unused_age: MaxUnusedAge,
) -> Option<&'a InstalledExecutable> {
    let line = version.line();
    seen.iter()
        .filter(|seen| {
            let entry = seen.entry;
            entry.kind == ExecutableKind::ProductExecutable
                && entry.product_version.line() == line
                && counts_as_installed(seen.still_at_path, entry.last_seen, now, max_unused_age)
                && (seen.still_at_path || seen.copy_present)
                && lowest_minor.is_none_or(|floor| entry.product_version.major_minor() >= floor)
        })
        .map(|seen| seen.entry)
        .max_by(|left, right| {
            rank(left.product)
                .cmp(&rank(right.product))
                .then_with(|| left.product_version.cmp(&right.product_version))
        })
}

/// The lowest Product version (major and minor) that can use every shared
/// store of the major of `version` in `home` that records one above the
/// major's first minor (ADR 0098), which the Daemon's pick must meet; none
/// when every store any minor can use, and during `0.y` previews, whose
/// line has the minor 0 alone. A store that cannot be read counts as
/// marked for every minor: its engine reports it when it opens it.
/// Blocking.
pub fn stores_floor(home: &Home, version: &ProductVersion) -> Option<ProtocolVersion> {
    if version.line().is_preview() {
        return None;
    }
    let engine = version.major_minor();
    let files = SettingsFiles::of(home, version);
    let settings = [files.operator_policy(), files.user_configuration()]
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let document: toml_edit::DocumentMut = text.parse().ok()?;
            document
                .get(LOWEST_MINOR_KEY)?
                .as_integer()
                .and_then(|minor| u16::try_from(minor).ok())
        });
    let run_index = run_index::lowest_minor(&home.run_index(version));
    let lowest = settings.chain(run_index).max()?;
    (lowest > 0).then_some(ProtocolVersion {
        major: engine.major,
        minor: lowest,
    })
}

/// What the Browser cache's clean-up reads of each entry (ADR 0098): every
/// entry that counts keeps its pins and its versioned copy.
pub fn kept_inputs(seen: &[Seen<'_>]) -> Vec<browser_cache::InstalledExecutable> {
    seen.iter()
        .map(|seen| browser_cache::InstalledExecutable {
            still_at_path: seen.still_at_path,
            last_seen: seen.entry.last_seen,
            pins: seen
                .entry
                .pins
                .iter()
                .chain(&seen.entry.copy)
                .cloned()
                .collect(),
        })
        .collect()
}

fn rank(product: Product) -> u8 {
    match product {
        Product::Collie => 0,
        Product::Foxhound => 1,
    }
}

#[cfg(test)]
mod tests;
