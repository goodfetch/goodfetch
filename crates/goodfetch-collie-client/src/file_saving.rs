//! The program's side of file saving (ADR 0116): a file written to the
//! program's path whole or not at all. Its bytes go into a hidden temporary
//! file beside the path, `.<name>.<random>.partial`, which is synced, then
//! renamed over the path, missing folders made first and a file there
//! replaced; a failure, and a [`FileSaving`] dropped before it finishes,
//! removes the temporary file, so the path holds the whole file or what it
//! held before. A failure the path causes ([`Error::PathNotWritable`]) is
//! told from one of the storage ([`Error::FileWriteFailed`]) by its
//! `io::ErrorKind`, and the OS's error is kept as the source. The engine
//! writes a kept file under the same rule; this one writes the bytes a
//! Daemon streams. Its calls block: [`crate::Connection::save_file`] runs
//! them on the Connection's tracker.
#![expect(
    clippy::disallowed_methods,
    reason = "its calls block: the Connection runs them as blocking tasks on its tracker"
)]

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::error::{Error, FileFailure};

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

/// A file on its way to the program's path: its bytes go into the hidden
/// temporary file beside the path, which [`FileSaving::finish`] syncs and
/// renames over it. Dropped before then, it removes the temporary file, so
/// the path holds what it held before.
#[derive(Debug)]
pub struct FileSaving {
    partial: tempfile::NamedTempFile,
    given: PathBuf,
    path: PathBuf,
    folder: PathBuf,
    call: String,
}

impl FileSaving {
    /// Starts the file for `path`, resolved against the working directory
    /// when it is relative: its missing folders made, then its hidden
    /// temporary file created beside it. `call` is the call that writes
    /// it, which a failure's fix names.
    ///
    /// # Errors
    ///
    /// [`Error::PathNotWritable`] when the path cannot hold a file, such
    /// as a folder or a path through a file, and [`Error::FileWriteFailed`]
    /// when its storage fails. Either names the path as given.
    pub fn create(path: &Path, call: &str) -> Result<Self, Error> {
        Self::create_at(path, absolute(path, call)?, call)
    }

    /// Starts the file for `given`, which resolves to the absolute `path`,
    /// as [`FileSaving::create`] does.
    pub(crate) fn create_at(given: &Path, path: PathBuf, call: &str) -> Result<Self, Error> {
        let named_folder = given
            .as_os_str()
            .to_string_lossy()
            .ends_with(std::path::is_separator);
        let name = match path.file_name() {
            Some(name) if !named_folder => name.to_owned(),
            _ => {
                return Err(failure(
                    "invalid_filename",
                    None,
                    given,
                    call,
                    "it names no file",
                    None,
                ));
            }
        };
        if std::fs::metadata(&path).is_ok_and(|found| found.is_dir()) {
            return Err(failure(
                "is_a_directory",
                unix_code("EISDIR"),
                given,
                call,
                "it is a folder",
                None,
            ));
        }
        let folder = path.parent().unwrap_or(Path::new("/")).to_owned();
        std::fs::create_dir_all(&folder).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                failure(
                    "not_a_directory",
                    unix_code("ENOTDIR"),
                    given,
                    call,
                    &format!(
                        "a file is where the folder `{}` should be",
                        folder.display()
                    ),
                    Some(error),
                )
            } else {
                classify(error, given, call, "make its folder")
            }
        })?;
        let mut prefix = std::ffi::OsString::from(".");
        prefix.push(&name);
        prefix.push(".");
        let partial = tempfile::Builder::new()
            .prefix(&prefix)
            .suffix(".partial")
            .make_in(&folder, create_hidden)
            .map_err(|error| classify(error, given, call, "create the temporary file"))?;
        Ok(Self {
            partial,
            given: given.to_owned(),
            path,
            folder,
            call: call.to_owned(),
        })
    }

    /// Writes `bytes` after those written before.
    ///
    /// # Errors
    ///
    /// [`Error::FileWriteFailed`] when the storage fails, such as when it
    /// is full, or [`Error::PathNotWritable`] for a file too large for it.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.partial
            .as_file_mut()
            .write_all(bytes)
            .map_err(|error| classify(error, &self.given, &self.call, "write the file"))
    }

    /// Syncs the file, then renames it over the path, and syncs its folder.
    ///
    /// # Errors
    ///
    /// As [`FileSaving::write`]'s; the path then holds what it held before.
    pub fn finish(self) -> Result<(), Error> {
        let Self {
            partial,
            given,
            path,
            folder,
            call,
        } = self;
        partial
            .as_file()
            .sync_all()
            .map_err(|error| classify(error, &given, &call, "sync the file"))?;
        unhide(partial.path()).map_err(|error| classify(error, &given, &call, "show the file"))?;
        partial
            .persist(&path)
            .map_err(|error| classify(error.error, &given, &call, "rename it over the path"))?;
        sync_folder(&folder);
        Ok(())
    }
}

/// `path`, resolved against the working directory when it is relative.
///
/// # Errors
///
/// When the working directory cannot be read, as `call`'s failure.
pub(crate) fn absolute(path: &Path, call: &str) -> Result<PathBuf, Error> {
    std::path::absolute(path).map_err(|error| {
        classify(
            error,
            path,
            call,
            "resolve it against the working directory",
        )
    })
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
        #[expect(
            unsafe_code,
            reason = "Windows offers the file attribute only through this call, and the standard library doesn't wrap it"
        )]
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that lives across
        // the call.
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

/// Syncs `folder`, where the OS allows it, so the renamed file survives a
/// power cut; one that cannot be synced is left as it is, the file itself
/// written already.
fn sync_folder(folder: &Path) {
    #[cfg(unix)]
    if let Ok(folder) = File::open(folder) {
        let _ = folder.sync_all();
    }
    #[cfg(not(unix))]
    let _ = folder;
}

/// The failure `error` of `doing` while `call` wrote `path`, the path's or
/// the storage's by its kind.
fn classify(error: io::Error, path: &Path, call: &str, doing: &str) -> Error {
    let reason = reason_of(error.kind());
    let os_code = error.raw_os_error().and_then(os_code);
    let detail = format!("could not {doing}: {error}");
    failure(reason, os_code, path, call, &detail, Some(error))
}

/// The error of `reason` while `call` wrote `path`, which `detail` tells.
fn failure(
    reason: &'static str,
    os_code: Option<&'static str>,
    path: &Path,
    call: &str,
    detail: &str,
    cause: Option<io::Error>,
) -> Error {
    let shown = path.display();
    if PATH_REASONS.contains(&reason) {
        let fix = match reason {
            "is_a_directory" => format!("Give `{call}` a file name, not a folder."),
            "not_a_directory" => {
                format!("Give `{call}` a path whose parent folders are folders, not files.")
            }
            "invalid_filename" => format!("Give `{call}` a file name this filesystem accepts."),
            "file_too_large" => {
                format!("Give `{call}` a path on a filesystem that holds a file this size.")
            }
            _ => format!("Give `{call}` a path in a folder this user can write."),
        };
        Error::PathNotWritable(Box::new(FileFailure {
            reason,
            os_code,
            side: None,
            path: path.to_owned(),
            message: format!(
                "The path `{shown}` cannot be written ({detail}); nothing was written there."
            ),
            fix,
            cause,
        }))
    } else {
        Error::FileWriteFailed(Box::new(FileFailure {
            reason,
            os_code,
            side: Some("program"),
            path: path.to_owned(),
            message: format!(
                "The file could not be written at `{shown}` ({detail}); the path holds what it held before."
            ),
            fix: format!("Free space where `{shown}` is, then call `{call}` again."),
            cause,
        }))
    }
}

/// `name`, the OS's name of an error found before the OS gives one, on the
/// OSes that name it so.
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
fn reason_of(kind: io::ErrorKind) -> &'static str {
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
fn os_code(code: i32) -> Option<&'static str> {
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
