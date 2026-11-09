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

/// The reasons ADR 0116 gives `path_not_writable`; every other one is the
/// storage's.
const PATH_REASONS: [&str; 7] = [
    "permission_denied",
    "read_only_filesystem",
    "is_a_directory",
    "not_a_directory",
    "invalid_filename",
    "file_too_large",
    "not_found",
];

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
    /// Starts the file for `path`: its missing folders made, then its
    /// hidden temporary file created beside it.
    ///
    /// # Errors
    ///
    /// [`SaveError::Path`] when the path cannot hold a file, such as a
    /// folder or a path through a file, and [`SaveError::Storage`] when its
    /// storage fails.
    pub fn create(path: &Path) -> Result<Self, SaveError> {
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

/// Creates the temporary file at `path`, hidden on Windows, where a leading
/// dot hides nothing.
fn create_hidden(path: &Path) -> io::Result<File> {
    let mut options = File::options();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.attributes(windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN);
    }
    options.open(path)
}

/// Clears the hidden attribute of the temporary file at `path` before it
/// becomes the program's file, on Windows; elsewhere its name hides it.
#[cfg_attr(
    not(windows),
    expect(clippy::unnecessary_wraps, reason = "it fails only on Windows")
)]
fn unhide(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that lives across
        // the call.
        #[expect(
            unsafe_code,
            reason = "Windows offers the file attribute only through this call, and the standard library doesn't wrap it"
        )]
        let set = unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                wide.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

/// The failure `error` of `doing`, the path's or the storage's by its kind.
fn classify(error: &io::Error, doing: &str) -> SaveError {
    let reason = reason_of(error.kind());
    let os_code = error.raw_os_error().and_then(os_code);
    let message = format!("could not {doing}: {error}");
    if PATH_REASONS.contains(&reason) {
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

/// `name`, the OS's name of an error the engine finds before the OS gives
/// one, on the OSes that name it so.
fn unix_code(name: &'static str) -> Option<&'static str> {
    #[cfg(unix)]
    let code = Some(name);
    #[cfg(not(unix))]
    let code = {
        let _ = name;
        None
    };
    code
}

/// `kind`'s name in `snake_case`, as ADR 0116 reports it.
pub(crate) fn reason_of(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::StorageFull => "storage_full",
        io::ErrorKind::QuotaExceeded => "quota_exceeded",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::ReadOnlyFilesystem => "read_only_filesystem",
        io::ErrorKind::IsADirectory => "is_a_directory",
        io::ErrorKind::NotADirectory => "not_a_directory",
        io::ErrorKind::InvalidFilename => "invalid_filename",
        io::ErrorKind::FileTooLarge => "file_too_large",
        io::ErrorKind::NotFound => "not_found",
        _ => "other",
    }
}

/// The OS's own name of the error numbered `code`, for the errors a file
/// write meets.
pub(crate) fn os_code(code: i32) -> Option<&'static str> {
    #[cfg(unix)]
    let name = match code {
        1 => Some("EPERM"),
        2 => Some("ENOENT"),
        5 => Some("EIO"),
        13 => Some("EACCES"),
        20 => Some("ENOTDIR"),
        21 => Some("EISDIR"),
        27 => Some("EFBIG"),
        28 => Some("ENOSPC"),
        30 => Some("EROFS"),
        #[cfg(target_os = "linux")]
        122 => Some("EDQUOT"),
        #[cfg(target_vendor = "apple")]
        69 => Some("EDQUOT"),
        _ => None,
    };
    #[cfg(windows)]
    let name = match code {
        2 => Some("ERROR_FILE_NOT_FOUND"),
        3 => Some("ERROR_PATH_NOT_FOUND"),
        5 => Some("ERROR_ACCESS_DENIED"),
        19 => Some("ERROR_WRITE_PROTECT"),
        39 => Some("ERROR_HANDLE_DISK_FULL"),
        112 => Some("ERROR_DISK_FULL"),
        _ => None,
    };
    #[cfg(not(any(unix, windows)))]
    let name = {
        let _ = code;
        None
    };
    name
}
