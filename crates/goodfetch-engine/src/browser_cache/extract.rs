//! Unpacks a downloaded ZIP archive, the format Chrome for Testing ships,
//! into an install's `files/` folder, or places a downloaded single
//! executable there, the format Lightpanda ships (ADR 0020). The zip crate
//! refuses entries whose paths leave the folder, and keeps Unix permissions
//! and links. Runs on the tasks module's blocking policy.
#![expect(
    clippy::disallowed_methods,
    reason = "unpacking reads and writes files on the tasks module's blocking policy, never on an async worker"
)]

use std::fs::File;
use std::io;
use std::path::Path;

/// Unpacks the archive at `archive` into `into`, then deletes the archive.
pub(super) fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    let mut reader = zip::ZipArchive::new(File::open(archive)?).map_err(invalid)?;
    reader.extract(into).map_err(invalid)?;
    drop(reader);
    std::fs::remove_file(archive)
}

/// Moves the single executable downloaded at `download` into `into` as
/// `name`, executable by everyone on Unix. A name that is no plain file
/// name is refused.
pub(super) fn place(download: &Path, into: &Path, name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a pinned executable's URL names no file",
        ));
    }
    std::fs::create_dir_all(into)?;
    let placed = into.join(name);
    std::fs::rename(download, &placed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&placed, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn invalid(error: zip::result::ZipError) -> io::Error {
    match error {
        zip::result::ZipError::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}
