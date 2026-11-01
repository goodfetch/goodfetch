//! Folder syncs that keep a name through a power cut (ADR 0002): a file's
//! own sync keeps its bytes, and only its folder's sync keeps the entry
//! that names them, as a file created or renamed into it, or a folder made
//! in it. Each call blocks: run it under its owner's blocking policy.

use std::fs::DirBuilder;
use std::io::{self, Write as _};
use std::path::Path;

/// Replaces the file at `path` with `bytes`, through a synced temporary
/// file renamed over it, so a reader never sees half of one, then syncs its
/// folder, so the file survives a power cut once this answers. The file is
/// its owner's alone (`0600`) on Unix, from its creation on. Its folder
/// must exist ([`create_folder`] makes one).
///
/// # Errors
///
/// When the file cannot be written or renamed into place.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    sync_folder(folder);
    Ok(())
}

/// Syncs the folder `folder`, where the OS allows it, so a file created or
/// renamed into it survives a power cut. A folder that cannot be opened or
/// synced is left as it is: the file itself is written already.
pub(crate) fn sync_folder(folder: &Path) {
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "it blocks: its callers run it under their owner's blocking policy"
    )]
    if let Ok(folder) = std::fs::File::open(folder) {
        let _ = folder.sync_all();
    }
    #[cfg(not(unix))]
    let _ = folder;
}

/// Creates `folder` and each missing folder above it, then syncs the folder
/// above each one it made, so the new folders survive a power cut as the
/// files written in them do.
///
/// # Errors
///
/// When a folder cannot be created.
pub(crate) fn create_folder(folder: &Path) -> io::Result<()> {
    let missing: Vec<&Path> = folder
        .ancestors()
        .take_while(|level| !level.as_os_str().is_empty() && !level.is_dir())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    DirBuilder::new().recursive(true).create(folder)?;
    for made in missing.iter().rev() {
        if let Some(above) = made.parent() {
            sync_folder(above);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
