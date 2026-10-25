//! Folder syncs that keep a name through a power cut (ADR 0002): a file's
//! own sync keeps its bytes, and only its folder's sync keeps the entry
//! that names them, as a file created or renamed into it, or a folder made
//! in it. Each call blocks: run it under its owner's blocking policy.

use std::fs::DirBuilder;
use std::io;
use std::path::Path;

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
