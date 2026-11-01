//! A filesystem's readings on Unix (ADR 0122): `statvfs`'s space and inodes
//! available to the engine's user (`f_bavail`, `f_favail`), and the device
//! that tells filesystems apart. System calls that Miri cannot run: the tests
//! on a real OS cover them (STYLE.md, `unsafe`).
#![expect(
    unsafe_code,
    reason = "Unix gives a filesystem's space and inodes only through `statvfs` (ADR 0122)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "every read here runs under the governor's blocking policy (STYLE.md)"
)]

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use super::{FilesystemInodes, FilesystemReading};

/// The device of the filesystem holding `folder`, which exists.
pub(super) fn device(folder: &Path) -> Option<u64> {
    std::fs::metadata(folder)
        .ok()
        .map(|metadata| metadata.dev())
}

/// What the filesystem holding `folder`, which exists, reads.
pub(super) fn read(folder: &Path) -> Option<FilesystemReading> {
    let path = CString::new(folder.as_os_str().as_bytes()).ok()?;
    let mut found = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `path` is a NUL-terminated string and `found` a writable
    // `statvfs` the call fills.
    let done = unsafe { libc::statvfs(path.as_ptr(), found.as_mut_ptr()) };
    if done != 0 {
        return None;
    }
    // SAFETY: the call succeeded, so it filled `found`.
    let found = unsafe { found.assume_init() };
    let unit = wide(found.f_frsize);
    let total = wide(found.f_files);
    Some(FilesystemReading {
        size: wide(found.f_blocks).saturating_mul(unit),
        free: wide(found.f_bavail).saturating_mul(unit),
        inodes: (total > 0).then(|| FilesystemInodes {
            total,
            free: wide(found.f_favail),
        }),
    })
}

/// A `statvfs` count, whose width differs by OS, as a `u64`.
fn wide<T: Into<u64>>(count: T) -> u64 {
    count.into()
}
