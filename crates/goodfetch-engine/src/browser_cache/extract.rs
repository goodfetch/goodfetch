//! Unpacks a downloaded ZIP archive, the format Chrome for Testing ships,
//! into an install's `files/` folder. The zip crate refuses entries whose
//! paths leave the folder, and keeps Unix permissions and links. Runs on the
//! tasks module's blocking policy.
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

fn invalid(error: zip::result::ZipError) -> io::Error {
    match error {
        zip::result::ZipError::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}
