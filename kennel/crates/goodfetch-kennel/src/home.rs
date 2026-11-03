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
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::daemon::DaemonRuntimeFile;

/// The folder the home is under the OS's data folder when
/// `GOODFETCH_HOME` is unset, as the Products name it.
const HOME_FOLDER: &str = "goodfetch";
/// The Kennel's own settings file, in the home (ADR 0141).
const SETTINGS_FILE: &str = "kennel.toml";
/// The list each Product's executable keeps of the installed executables
/// (ADR 0098).
const INSTALLED_EXECUTABLES_FILE: &str = "installed-executables.json";
/// The Daemon's runtime file, in its per-major folder (ADR 0024).
const RUNTIME_FILE: &str = "daemon.json";
/// The lock a serving Daemon holds beside its runtime file.
const SERVING_LOCK: &str = "daemon.lock";

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
        self.0.join(INSTALLED_EXECUTABLES_FILE)
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
                let text = fs::read(folder.join(RUNTIME_FILE)).ok()?;
                let runtime = serde_json::from_slice::<DaemonRuntimeFile>(&text).ok()?;
                Some((order, runtime))
            })
            .max_by_key(|(order, _)| *order)
            .map(|(_, runtime)| runtime)
    }
}

/// The per-major folders in the home at `home`, each with its major's
/// order; none when the home cannot be read. Blocks.
pub(crate) fn major_folders(home: &Path) -> Vec<((u64, u64), PathBuf)> {
    let Ok(entries) = fs::read_dir(home) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            Some((
                major_order(&entry.file_name().to_string_lossy())?,
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
    name == RUNTIME_FILE
        || name
            .to_str()
            .is_some_and(|name| major_order(name).is_some())
}

/// Whether a Daemon holds the serving lock in `folder`: taking it fails
/// with `WouldBlock`. A lock this check takes is let go at once.
fn serving_lock_held(folder: &Path) -> bool {
    let Ok(lock) = File::open(folder.join(SERVING_LOCK)) else {
        return false;
    };
    matches!(lock.try_lock(), Err(TryLockError::WouldBlock))
}

/// The order of a per-major folder's name, `1` or `0.4`, by which majors
/// compare: `None` for a name that is no major's (the engine's rule).
fn major_order(name: &str) -> Option<(u64, u64)> {
    let number = |part: &str| {
        let canonical = !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'));
        canonical.then(|| part.parse().ok()).flatten()
    };
    match name.split_once('.') {
        Some(("0", minor)) => Some((0, number(minor)?)),
        Some(_) => None,
        None => match number(name)? {
            0 => None,
            major => Some((major, 0)),
        },
    }
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
