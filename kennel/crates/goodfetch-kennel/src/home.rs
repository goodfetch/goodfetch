//! The Goodfetch home and what the Kennel reads in it: the Daemon's runtime
//! file and serving lock in each per-major folder, the installed
//! executables' list and the Kennel's own settings file.
//!
//! Every read here blocks, so the Kennel makes them on the tracker's
//! blocking threads (`tasks`).
#![expect(
    clippy::disallowed_methods,
    reason = "the home's files are read on the tracker's blocking threads, through `Tasks::blocking` (STYLE.md)"
)]

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::VersionLine;
use goodfetch_protocol::daemon::DaemonRuntimeFile;
use goodfetch_protocol::home::{DaemonFile, HOME_FOLDER, INSTALLED_EXECUTABLES};

/// The Kennel's own settings file, in the home (ADR 0141).
const SETTINGS_FILE: &str = "kennel.toml";

/// The Goodfetch home.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Home(PathBuf);

impl Home {
    /// The home at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// The home the Products use: `variable`, the value of
    /// `GOODFETCH_HOME`, when it is set and not empty, else the `goodfetch`
    /// folder of the OS's data folder; none when the OS names no data
    /// folder.
    pub fn resolve(variable: Option<OsString>) -> Option<Self> {
        match variable {
            Some(path) if !path.is_empty() => Some(Self::new(path)),
            _ => dirs::data_dir().map(|data| Self::new(data.join(HOME_FOLDER))),
        }
    }

    /// Its folder.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The Kennel's settings file.
    pub fn settings_file(&self) -> PathBuf {
        self.0.join(SETTINGS_FILE)
    }

    /// The installed executables' list.
    pub fn installed_executables_file(&self) -> PathBuf {
        self.0.join(INSTALLED_EXECUTABLES)
    }

    /// The running Daemon's runtime file. Each per-major folder may hold a
    /// Daemon (ADR 0098); a Daemon runs only while its runtime file exists
    /// and its serving lock is held, as the Products' Clients judge it, so
    /// a file a dead Daemon left is passed over. When several run, the
    /// newest major's wins. Blocks.
    pub fn running_daemon(&self) -> Option<DaemonRuntimeFile> {
        major_folders(&self.0)
            .into_iter()
            .filter_map(|(order, folder)| {
                if !serving_lock_held(&folder) {
                    return None;
                }
                let text = fs::read(folder.join(DaemonFile::RuntimeFile.name())).ok()?;
                let runtime = serde_json::from_slice::<DaemonRuntimeFile>(&text).ok()?;
                Some((order, runtime))
            })
            .max_by_key(|(order, _)| *order)
            .map(|(_, runtime)| runtime)
    }
}

/// The per-major folders in the home at `home`, each with its line; none
/// when the home cannot be read. Blocks.
pub(crate) fn major_folders(home: &Path) -> Vec<(VersionLine, PathBuf)> {
    let Ok(entries) = fs::read_dir(home) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            Some((
                VersionLine::parse(&entry.file_name().to_string_lossy())?,
                entry.path(),
            ))
        })
        .collect()
}

/// Whether a change to the entry `name` of the home or of a per-major
/// folder can change which Daemon runs: a per-major folder, or a runtime
/// file, which a Daemon writes when it starts, by rename, and removes
/// when it stops.
pub(crate) fn names_a_daemons_file(name: &OsStr) -> bool {
    name == DaemonFile::RuntimeFile.name()
        || name
            .to_str()
            .is_some_and(|name| VersionLine::parse(name).is_some())
}

/// Whether a Daemon holds the serving lock in `folder`: taking it fails
/// with `WouldBlock`. A lock this check takes is let go at once.
fn serving_lock_held(folder: &Path) -> bool {
    let Ok(lock) = File::open(folder.join(DaemonFile::ServingLock.name())) else {
        return false;
    };
    matches!(lock.try_lock(), Err(TryLockError::WouldBlock))
}

/// `folder`'s canonical path, or `folder` when it has none. Blocks.
pub(crate) fn canonical(folder: &Path) -> PathBuf {
    fs::canonicalize(folder).unwrap_or_else(|_| folder.to_owned())
}

/// Reads the file at `path`, or none when it does not exist. Blocks.
pub(crate) fn read_if_present(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Replaces the file at `path` with `bytes`, safely: they are written to a
/// temporary file in the same folder, which then takes the file's place by
/// rename, so a reader sees the old contents or the new, never a part.
/// The folder is made when missing. Blocks.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(folder)?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}
