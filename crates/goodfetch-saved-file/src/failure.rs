//! What a failed write of a Saved file reports (ADR 0116): its reason, the
//! OS's own name of the error, and whether the reason blames the path the
//! program chose (`path_not_writable`) or the storage (`file_write_failed`).

use std::io;

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

/// Whether `reason` blames the path the program chose, `path_not_writable`,
/// rather than the storage, `file_write_failed`.
#[must_use]
pub fn blames_path(reason: &str) -> bool {
    PATH_REASONS.contains(&reason)
}

/// `name`, the OS's name of an error found before the OS gives one, on the
/// OSes that name it so.
#[must_use]
pub fn unix_code(name: &'static str) -> Option<&'static str> {
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
#[must_use]
pub fn reason_of(kind: io::ErrorKind) -> &'static str {
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
#[must_use]
pub fn os_code(code: i32) -> Option<&'static str> {
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
