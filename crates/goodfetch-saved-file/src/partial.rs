//! The hidden temporary file a save writes beside its path,
//! `.<name>.<random>.partial`: created hidden, and shown again before it is
//! renamed over the path. Its calls block: their callers run them under
//! their owner's blocking policy.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet, under its caller's blocking policy; the Shared paths audit moves it onto its counted function"
)]

use std::fs::File;
use std::io;
use std::path::Path;

/// Creates the temporary file at `path`, hidden on Windows, where a leading
/// dot hides nothing.
///
/// # Errors
///
/// The OS's own, such as when a file is already at `path`.
pub fn create_hidden(path: &Path) -> io::Result<File> {
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
///
/// # Errors
///
/// The OS's own, on Windows only.
pub fn unhide(path: &Path) -> io::Result<()> {
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
