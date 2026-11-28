//! A kept file written to the program's path under ADR 0116's rule: into a
//! hidden temporary file beside the path, `.<name>.<random>.partial`, which
//! is synced, then renamed over the path, missing folders made first and a
//! file there replaced. Any failure, and a cancellation, removes the
//! temporary file, so the path holds the whole file or what it held
//! before. A failure the path causes is told from one of the storage by
//! `io::ErrorKind`. [`Saving`] writes a path the same way from bytes that
//! arrive in parts. Its calls block: run them under the owner's blocking
//! policy.
#![expect(
    clippy::disallowed_methods,
    reason = "its calls block: their callers run them under their owner's blocking policy"
)]

use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_saved_file::{
    blames_path, check_save_path, create_hidden, os_code, reason_of, resolve_in_folder, unhide,
    unix_code,
};

use crate::durable::sync_folder;

/// Why a kept file did not reach the program's path, which holds what it
/// held before.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The path cannot be written for a reason the path causes:
    /// `path_not_writable`.
    #[error("{message}")]
    Path {
        /// ADR 0116's reason, such as `permission_denied`.
        reason: &'static str,
        /// The OS's name of the error, where it gives one.
        os_code: Option<&'static str>,
        /// What failed, for people.
        message: String,
    },
    /// The program's storage failed: `file_write_failed`, side `program`.
    #[error("{message}")]
    Storage {
        /// ADR 0116's reason, such as `storage_full`.
        reason: &'static str,
        /// The OS's name of the error, where it gives one.
        os_code: Option<&'static str>,
        /// What failed, for people.
        message: String,
    },
    /// The kept file could not be read where the engine keeps it:
    /// `file_write_failed`, side `engine`.
    #[error("{message}")]
    Source {
        /// ADR 0116's reason.
        reason: &'static str,
        /// The OS's name of the error, where it gives one.
        os_code: Option<&'static str>,
        /// What failed, for people.
        message: String,
    },
    /// The call was cancelled before the rename.
    #[error("the save was cancelled")]
    Cancelled,
}

impl SaveError {
    /// The storage's failure `error` while it did `doing`, whatever its
    /// kind, such as a write to standard output, which has no path to
    /// blame: `file_write_failed`, side `program`.
    pub fn of_storage(error: &io::Error, doing: &str) -> Self {
        Self::Storage {
            reason: reason_of(error.kind()),
            os_code: error.raw_os_error().and_then(os_code),
            message: format!("could not {doing}: {error}"),
        }
    }
}

/// Writes the file at `source` to `path`, whole or not at all, giving up
/// before the rename once `cancelled` answers true.
///
/// # Errors
///
/// As [`SaveError`] says; none leaves a file beside `path` or changes it.
pub(crate) fn save(
    source: &Path,
    path: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), SaveError> {
    let mut saving = Saving::create(path)?;
    let mut from = File::open(source).map_err(|error| SaveError::Source {
        reason: reason_of(error.kind()),
        os_code: error.raw_os_error().and_then(os_code),
        message: format!("could not read the kept file: {error}"),
    })?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(SaveError::Cancelled);
        }
        let read = match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(SaveError::Source {
                    reason: reason_of(error.kind()),
                    os_code: error.raw_os_error().and_then(os_code),
                    message: format!("could not read the kept file: {error}"),
                });
            }
        };
        saving.write(&buffer[..read])?;
    }
    if cancelled() {
        return Err(SaveError::Cancelled);
    }
    saving.finish()
}

/// A file on its way to a path under ADR 0116's rule, whatever gives its
/// bytes, a kept file or the `$/chunk` notifications a CLI command
/// receives: they go into the hidden temporary file beside the path, which
/// [`Saving::finish`] syncs and renames over it. Dropped before then, it
/// removes the temporary file, so the path holds what it held before.
pub struct Saving {
    partial: tempfile::NamedTempFile,
    path: PathBuf,
    folder: PathBuf,
}

impl Saving {
    /// Checks that `path` is safe to write across OSes before resolving it.
    ///
    /// # Errors
    ///
    /// [`SaveError::Path`] with `reason: "invalid_filename"` when `path`
    /// contains a traversal, reserved name or other unsafe segment.
    pub fn check_path(path: &Path) -> Result<(), SaveError> {
        check_save_path(path).map_err(|issue| SaveError::Path {
            reason: "invalid_filename",
            os_code: None,
            message: format!(
                "`{}` cannot be written: {}",
                path.display(),
                issue.describe()
            ),
        })
    }

    /// Resolves a file `name` inside `folder` and starts saving there,
    /// refusing any name that leaves `folder` or is unsafe on any OS.
    ///
    /// # Errors
    ///
    /// [`SaveError::Path`] (`invalid_filename`) for an unsafe name, or as
    /// [`Saving::create`].
    pub fn create_in_folder(folder: &Path, name: &str) -> Result<Self, SaveError> {
        let resolved = resolve_in_folder(folder, name).map_err(|issue| SaveError::Path {
            reason: "invalid_filename",
            os_code: None,
            message: format!("`{name}` cannot be written: {}", issue.describe()),
        })?;
        Self::create(&resolved)
    }

    /// Starts the file for `path`: its missing folders made, then its
    /// hidden temporary file created beside it.
    ///
    /// # Errors
    ///
    /// [`SaveError::Path`] when the path cannot hold a file, such as a
    /// folder or a path through a file, and [`SaveError::Storage`] when its
    /// storage fails.
    pub fn create(path: &Path) -> Result<Self, SaveError> {
        Self::check_path(path)?;
        let Some(name) = path.file_name() else {
            return Err(SaveError::Path {
                reason: "invalid_filename",
                os_code: None,
                message: format!("`{}` names no file", path.display()),
            });
        };
        if std::fs::metadata(path).is_ok_and(|found| found.is_dir()) {
            return Err(SaveError::Path {
                reason: "is_a_directory",
                os_code: unix_code("EISDIR"),
                message: format!("`{}` is a folder", path.display()),
            });
        }
        let folder = match path.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder,
            _ => Path::new("."),
        };
        std::fs::create_dir_all(folder).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                SaveError::Path {
                    reason: "not_a_directory",
                    os_code: unix_code("ENOTDIR"),
                    message: format!(
                        "a file is where the folder `{}` should be",
                        folder.display()
                    ),
                }
            } else {
                classify(&error, "make its folder")
            }
        })?;
        let mut prefix = std::ffi::OsString::from(".");
        prefix.push(name);
        prefix.push(".");
        let partial = tempfile::Builder::new()
            .prefix(&prefix)
            .suffix(".partial")
            .make_in(folder, create_hidden)
            .map_err(|error| classify(&error, "create the temporary file"))?;
        Ok(Self {
            partial,
            path: path.to_owned(),
            folder: folder.to_owned(),
        })
    }

    /// Writes `bytes` after those written before.
    ///
    /// # Errors
    ///
    /// [`SaveError::Storage`] when the storage fails, such as when it is
    /// full, or [`SaveError::Path`] for a file too large for it.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), SaveError> {
        self.partial
            .as_file_mut()
            .write_all(bytes)
            .map_err(|error| classify(&error, "write the file"))
    }

    /// Syncs the file, then renames it over the path, and syncs its folder.
    ///
    /// # Errors
    ///
    /// As [`Saving::write`]'s; the path then holds what it held before.
    pub fn finish(self) -> Result<(), SaveError> {
        self.partial
            .as_file()
            .sync_all()
            .map_err(|error| classify(&error, "sync the file"))?;
        unhide(self.partial.path()).map_err(|error| classify(&error, "show the file"))?;
        self.partial
            .persist(&self.path)
            .map_err(|error| classify(&error.error, "rename it over the path"))?;
        sync_folder(&self.folder);
        Ok(())
    }
}

/// The failure `error` of `doing`, the path's or the storage's by its kind.
fn classify(error: &io::Error, doing: &str) -> SaveError {
    let reason = reason_of(error.kind());
    let os_code = error.raw_os_error().and_then(os_code);
    let message = format!("could not {doing}: {error}");
    if blames_path(reason) {
        SaveError::Path {
            reason,
            os_code,
            message,
        }
    } else {
        SaveError::Storage {
            reason,
            os_code,
            message,
        }
    }
}
