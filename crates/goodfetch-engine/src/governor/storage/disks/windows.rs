//! A filesystem's readings on Windows (ADR 0122): `GetDiskFreeSpaceExW`'s
//! space available to the caller, which honours quotas, and the volume that
//! tells filesystems apart. Windows reports no inode count. System calls that
//! Miri cannot run: the tests on a real Windows cover them (STYLE.md,
//! `unsafe`).
#![expect(
    unsafe_code,
    reason = "Windows gives a volume's free space only through its C calls (ADR 0122)"
)]

use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumePathNameW};

use super::FilesystemReading;

/// The longest volume path `GetVolumePathNameW` writes, in UTF-16 units,
/// past `MAX_PATH` for a long path.
const VOLUME_PATH: usize = 32_768;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// The volume holding `folder`, which exists, as its mount point.
pub(super) fn volume(folder: &Path) -> Option<String> {
    let path = wide(folder);
    let mut volume = vec![0_u16; VOLUME_PATH];
    let length = u32::try_from(volume.len()).ok()?;
    // SAFETY: `path` is NUL-terminated and `volume` a writable buffer of
    // `length` UTF-16 units.
    let done = unsafe { GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), length) };
    if done == 0 {
        return None;
    }
    let end = volume.iter().position(|unit| *unit == 0)?;
    Some(String::from_utf16_lossy(&volume[..end]).to_lowercase())
}

/// What the volume holding `folder`, which exists, reads.
pub(super) fn read(folder: &Path) -> Option<FilesystemReading> {
    let path = wide(folder);
    let (mut free, mut size, mut all_free) = (0_u64, 0_u64, 0_u64);
    // SAFETY: `path` is NUL-terminated and each count a writable `u64`.
    let done = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &raw mut free,
            &raw mut size,
            &raw mut all_free,
        )
    };
    (done != 0).then_some(FilesystemReading {
        size,
        free,
        inodes: None,
    })
}
