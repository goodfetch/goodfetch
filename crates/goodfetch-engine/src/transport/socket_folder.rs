//! Where the engine's own Unix sockets go when no folder of the user's
//! choosing holds them: a folder of their own, mode `0700`, in the OS's
//! temporary folder, or in `/tmp` when a socket's path under that folder
//! would pass the OS's limit, as a deep `TMPDIR` makes it. A browser the
//! engine starts puts its own socket in its temporary folder and stops
//! when that path is too long, so it is given `/tmp` in the same case.

use std::io;
use std::path::Path;

/// The longest socket path every OS takes: macOS's `sun_path` holds 104
/// bytes, its last a NUL; Linux's holds 108.
pub const LONGEST_SOCKET_PATH: usize = 103;

/// The short temporary folder every macOS and Linux has, where any socket
/// the engine or its browser makes fits.
#[cfg(unix)]
const SHORT_TEMPORARY_FOLDER: &str = "/tmp";

/// How many random characters `tempfile` puts after a folder's prefix.
const RANDOM_CHARACTERS: usize = 6;

/// The temporary folder under which a socket whose path adds `below`
/// bytes to it fits the OS's limit: `temporary` when it does, otherwise
/// `/tmp` on macOS and Linux. Windows has no socket paths, so `temporary`
/// there.
pub fn temporary_folder_for_sockets(temporary: &Path, below: usize) -> &Path {
    #[cfg(unix)]
    if temporary.as_os_str().len() + below > LONGEST_SOCKET_PATH {
        return Path::new(SHORT_TEMPORARY_FOLDER);
    }
    #[cfg(not(unix))]
    let _ = below;
    temporary
}

/// A new folder of its own, mode `0700`, named `prefix` and random
/// characters, for the socket named `socket`: in `temporary` when the
/// socket's path fits the OS's limit there, otherwise in `/tmp`. Removed
/// when the answered guard drops.
///
/// # Errors
///
/// When the folder cannot be made.
pub fn socket_folder_in(
    temporary: &Path,
    prefix: &str,
    socket: &str,
) -> io::Result<tempfile::TempDir> {
    let below = 1 + prefix.len() + RANDOM_CHARACTERS + 1 + socket.len();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(temporary_folder_for_sockets(temporary, below))
}

/// [`socket_folder_in`] the OS's temporary folder.
///
/// # Errors
///
/// When the folder cannot be made.
pub fn socket_folder(prefix: &str, socket: &str) -> io::Result<tempfile::TempDir> {
    socket_folder_in(&std::env::temp_dir(), prefix, socket)
}

#[cfg(test)]
#[cfg(unix)]
mod tests;
