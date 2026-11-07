//! `upgrade-files` (ADRs 0030 and 0066): gives the executable's major its
//! own copy of an older major's per-user and per-install files in Goodfetch
//! home (its folder of that major: the Operator policy, user configuration,
//! the Run index and the run folders), and never changes the older major's
//! files, so that a person can go back. Its own Daemon's files, every
//! `DaemonFile` its tokens among them, are no state, and are not copied. Shared files outside the per-major folders
//! (the Browser cache, the Usage ledger's family, the list of installed
//! executables) need no copy; the Usage ledger's managed rows of older
//! majors are replaced with the new major's (ADR 0041).
//!
//! It upgrades only from a major `compat/upgrade-paths.toml` lists: another
//! older major is refused, naming the release to upgrade through first, and
//! a newer one is refused with a message to upgrade. During `0.y` previews
//! each `0.y` minor counts as a major (ADR 0030).
//!
//! Each file is copied whole into a temporary file beside its place in the
//! new major's folder, then renamed into it, so a cut-off run leaves no
//! half-copied file. A file the new major already holds is kept: one equal
//! to the older major's, as after an earlier run, is reported unchanged, so
//! a second run changes nothing. A format whose newer major reads its older
//! one differently converts its copy here, in the release that changes it;
//! none does yet.
#![expect(
    clippy::disallowed_methods,
    reason = "`upgrade-files` copies files on its command's own thread, before any runtime starts"
)]

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::{ProductVersion, VersionLine};
use goodfetch_protocol::home::DaemonFile;
use toml_edit::DocumentMut;

use crate::home::Home;
use crate::ledger::Ledger;

/// The committed table of the majors `upgrade-files` upgrades from.
const COMMITTED: &str = include_str!("../../../compat/upgrade-paths.toml");

/// The majors `upgrade-files` upgrades from, as `compat/upgrade-paths.toml`
/// lists them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpgradePaths {
    sources: Vec<String>,
}

impl UpgradePaths {
    /// The table this release ships.
    ///
    /// # Panics
    ///
    /// When the committed table does not read, which its Quick test rules
    /// out.
    pub fn committed() -> Self {
        Self::parse(COMMITTED).expect("invariant: the committed upgrade paths read")
    }

    /// Reads a table's text: one `[[source]]` per major, with its `major`.
    ///
    /// # Errors
    ///
    /// What does not read: no TOML, or a source that names no major.
    pub fn parse(text: &str) -> Result<Self, String> {
        let document: DocumentMut = text
            .parse()
            .map_err(|error: toml_edit::TomlError| error.message().to_owned())?;
        let mut sources = Vec::new();
        if let Some(listed) = document.get("source") {
            let tables = listed
                .as_array_of_tables()
                .ok_or("`source` is no array of tables")?;
            for table in tables {
                let major = table
                    .get("major")
                    .and_then(|major| major.as_str())
                    .filter(|major| VersionLine::parse(major).is_some())
                    .ok_or("a `source` names no `major`, such as \"1\" or \"0.4\"")?;
                sources.push(major.to_owned());
            }
        }
        if let Some(other) = document
            .iter()
            .map(|(name, _)| name)
            .find(|name| *name != "source")
        {
            return Err(format!("`{other}` is no part of the upgrade paths"));
        }
        Ok(Self { sources })
    }

    /// The majors it lists.
    pub fn sources(&self) -> &[String] {
        &self.sources
    }
}

/// Why `upgrade-files` changed nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum UpgradeError {
    /// An older major the table does not list.
    #[error("{}", not_listed(major, current, through.as_deref()))]
    NotListed {
        /// The major asked for.
        major: String,
        /// The executable's.
        current: String,
        /// The major whose release upgrades from `major`, to upgrade
        /// through first, when one comes before `current`.
        through: Option<String>,
    },
    /// A newer major than the executable's.
    #[error(
        "the files of major {major} are newer than this Goodfetch's major {current}; upgrade Goodfetch to major {major} or newer to use them"
    )]
    NewerMajor {
        /// The major asked for.
        major: String,
        /// The executable's.
        current: String,
    },
    /// `--from` names no major.
    #[error("`{major}` names no major, such as `1` or `0.4`")]
    NotAMajor {
        /// What was given.
        major: String,
    },
    /// A file could not be read or written; what was copied before it
    /// stays, and the older major's files are unchanged.
    #[error("`{path}` could not be copied: {message}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The OS's message.
        message: String,
    },
}

fn not_listed(source: &str, current: &str, through: Option<&str>) -> String {
    match through {
        Some(through) => format!(
            "this Goodfetch's major {current} does not upgrade the files of major {source}; upgrade them with Goodfetch major {through}'s `upgrade-files` first, then run this one"
        ),
        None => format!(
            "this Goodfetch's major {current} does not upgrade the files of major {source}, and no release lists it"
        ),
    }
}

/// What `upgrade-files` did: the major it copied from and to, and each
/// file of the new major's copy, by its path in Goodfetch home.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Upgraded {
    /// The older major it copied; none when Goodfetch home holds none.
    pub from: Option<String>,
    /// The executable's major.
    pub to: String,
    /// The files it copied.
    pub copied: Vec<PathBuf>,
    /// The files the new major already held as the older one has them.
    pub unchanged: Vec<PathBuf>,
    /// The files the new major already held otherwise.
    pub kept: Vec<PathBuf>,
}

/// Gives the major of `version` its copy of the files of `from`, or of the
/// newest older major Goodfetch home holds, under `paths`.
///
/// # Errors
///
/// [`UpgradeError`]'s refusals, before any file is written, and `Io` when a
/// file cannot be copied.
pub fn upgrade_files(
    home: &Home,
    version: &ProductVersion,
    from: Option<&str>,
    paths: &UpgradePaths,
) -> Result<Upgraded, UpgradeError> {
    let current_line = version.line();
    let current = current_line.to_string();
    let source = match from {
        Some(source) => {
            let line = VersionLine::parse(source).ok_or_else(|| UpgradeError::NotAMajor {
                major: source.to_owned(),
            })?;
            Some((line, source.to_owned()))
        }
        None => newest_older_major(home.root(), current_line),
    };
    let Some((line, source)) = source else {
        return Ok(Upgraded {
            to: current,
            ..Upgraded::default()
        });
    };
    if line > current_line {
        return Err(UpgradeError::NewerMajor {
            major: source,
            current,
        });
    }
    if line == current_line {
        return Ok(Upgraded {
            from: Some(source),
            to: current,
            ..Upgraded::default()
        });
    }
    if !paths.sources.contains(&source) {
        let through = line
            .next()
            .filter(|through| *through < current_line)
            .map(|through| through.to_string());
        return Err(UpgradeError::NotListed {
            major: source,
            current,
            through,
        });
    }
    let mut upgraded = Upgraded {
        from: Some(source.clone()),
        to: current.clone(),
        ..Upgraded::default()
    };
    let (from_folder, to_folder) = (home.root().join(&source), home.root().join(&current));
    for file in files_of(&from_folder)? {
        let relative = file
            .strip_prefix(&from_folder)
            .expect("invariant: a file found in a folder is under it")
            .to_owned();
        let target = to_folder.join(&relative);
        let shown = Path::new(&current).join(&relative);
        match std::fs::read(&target) {
            Ok(held) if held == read(&file)? => upgraded.unchanged.push(shown),
            Ok(_) => upgraded.kept.push(shown),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                copy(&file, &target)?;
                upgraded.copied.push(shown);
            }
            Err(error) => return Err(io_error(&target, &error)),
        }
    }
    let ledger = home.usage_ledger();
    if ledger.exists() {
        Ledger::at(ledger.clone(), None)
            .upgrade_managed(&source, &current, |major| {
                VersionLine::parse(major).is_some_and(|line| line < current_line)
            })
            .map_err(|error| UpgradeError::Io {
                path: ledger,
                message: error.to_string(),
            })?;
    }
    Ok(upgraded)
}

/// The newest major older than `current` whose folder Goodfetch home at
/// `root` holds.
fn newest_older_major(root: &Path, current: VersionLine) -> Option<(VersionLine, String)> {
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            VersionLine::parse(&name)
                .filter(|line| *line < current)
                .map(|line| (line, name))
        })
        .max()
}

/// Every regular file under `folder` but its Daemon's, in path order.
fn files_of(folder: &Path) -> Result<Vec<PathBuf>, UpgradeError> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(current) = folders.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| io_error(&current, &error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&current, &error))?;
            let kind = entry
                .file_type()
                .map_err(|error| io_error(&entry.path(), &error))?;
            let name = entry.file_name();
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file()
                && !(current == folder && name.to_str().and_then(DaemonFile::named).is_some())
            {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn read(path: &Path) -> Result<Vec<u8>, UpgradeError> {
    std::fs::read(path).map_err(|error| io_error(path, &error))
}

/// Copies `file` to `target` through a temporary file beside it, never
/// writing `file`.
fn copy(file: &Path, target: &Path) -> Result<(), UpgradeError> {
    let folder = target
        .parent()
        .expect("invariant: a file in the new major's folder has a parent");
    std::fs::create_dir_all(folder).map_err(|error| io_error(folder, &error))?;
    let mut partial = target.as_os_str().to_owned();
    partial.push(".upgrade-partial");
    let partial = PathBuf::from(partial);
    std::fs::copy(file, &partial).map_err(|error| io_error(&partial, &error))?;
    std::fs::rename(&partial, target).map_err(|error| io_error(target, &error))
}

fn io_error(path: &Path, error: &io::Error) -> UpgradeError {
    UpgradeError::Io {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests;
