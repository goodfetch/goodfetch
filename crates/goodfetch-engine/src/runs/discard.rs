//! A Run's discard (ADRs 0001, 0002, 0015 and 0036): what erases the Run's
//! own state, under its writer lock, so that nothing of it is left behind.
//! Its wrapped data key is overwritten in its run file first, with
//! `secure_delete` on and the run file's WAL truncated; in the per-install
//! layout the install database then deletes the run folder's files; the
//! run folder is removed last, with its run file, page bodies, browser
//! profiles and Checkpoints, the lock file after every other entry, since
//! Windows removes no file a handle holds. Its call blocks: the registry
//! runs it under its owner's blocking policy.
//!
//! The Run's Run index row is the registry's to mark erased. A Profile
//! directory a Run uses in place and a Borrowed browser are outside the run
//! folder, the person's own, and never touched (ADR 0056). No Response
//! cache, compiled Clean-up rules or `filter_lists.extra` list is kept
//! outside a run folder yet (ADRs 0046 and 0047): the ticket that keeps one
//! deletes the discarded Run's entries here.

use std::io;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension as _};

use super::RUN_FILE;
use super::writer::{LOCK_FILE, LockError, take_lock};
use crate::run_state::install::InstallDatabase;
use crate::run_state::{RunStateError, sqlite_error};

/// What a discard found of a Run.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Discarded {
    /// Its state was there, and is gone.
    Erased,
    /// Nothing of it was there.
    Absent,
}

/// Why a Run's state was not erased.
#[derive(Debug, thiserror::Error)]
pub(super) enum DiscardError {
    /// Another engine holds its writer lock.
    #[error("another engine holds the Run's writer lock")]
    Held,
    /// Its run file or the install database could not be written.
    #[error(transparent)]
    State(#[from] RunStateError),
    /// The OS refused the run folder or one of its files.
    #[error("the run folder could not be removed: {0}")]
    Io(String),
}

/// Erases the state of the Run `run_id` in `folder`, its run file opening
/// through the VFS `vfs`, and from `install` in the per-install layout.
pub(super) fn discard(
    run_id: &str,
    folder: &Path,
    vfs: Option<&str>,
    install: Option<&InstallDatabase>,
) -> Result<Discarded, DiscardError> {
    let folder_exists =
        std::fs::exists(folder).map_err(|error| DiscardError::Io(error.to_string()))?;
    let installed = match install {
        Some(install) => install.runs()?.iter().any(|run| run == run_id),
        None => false,
    };
    if !folder_exists && !installed {
        return Ok(Discarded::Absent);
    }
    let lock = match take_lock(folder) {
        Ok(lock) => lock,
        Err(LockError::Held) => return Err(DiscardError::Held),
        Err(LockError::Io(error)) => return Err(DiscardError::Io(error)),
    };
    destroy_key(&folder.join(RUN_FILE), vfs, installed)?;
    if let Some(install) = install {
        install.erase(run_id)?;
    }
    remove_all_but_the_lock(folder).map_err(|error| DiscardError::Io(error.to_string()))?;
    drop(lock);
    remove_folder(folder).map_err(|error| DiscardError::Io(error.to_string()))?;
    Ok(Discarded::Erased)
}

/// Overwrites the wrapped data key the run file at `file` holds, if any,
/// with `secure_delete` on, then truncates its WAL (ADR 0015).
fn destroy_key(file: &Path, vfs: Option<&str>, installed: bool) -> Result<(), RunStateError> {
    let on_disk = std::fs::exists(file).unwrap_or(false);
    if !on_disk && !installed {
        return Ok(());
    }
    let failed = |error: rusqlite::Error| sqlite_error(file, &error);
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = match vfs {
        Some(vfs) => Connection::open_with_flags_and_vfs(file, flags, vfs),
        None => Connection::open_with_flags(file, flags),
    };
    let connection = match connection {
        Ok(connection) => connection,
        // A run folder whose run file never came to be holds no key.
        Err(error) if error.sqlite_error_code() == Some(rusqlite::ErrorCode::CannotOpen) => {
            return Ok(());
        }
        Err(error) => return Err(failed(error)),
    };
    connection
        .pragma_update(None, "secure_delete", "ON")
        .map_err(failed)?;
    let keyed = connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'run_key'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(failed)?
        .is_some();
    if keyed {
        connection
            .execute("DELETE FROM run_key", [])
            .map_err(failed)?;
    }
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(failed)?;
    Ok(())
}

/// Removes every entry of `folder` but its lock file.
#[expect(
    clippy::disallowed_methods,
    reason = "runs under the registry owner's blocking policy"
)]
fn remove_all_but_the_lock(folder: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_name() == LOCK_FILE {
            continue;
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Removes `folder` and what is left in it, its lock file.
#[expect(
    clippy::disallowed_methods,
    reason = "runs under the registry owner's blocking policy"
)]
fn remove_folder(folder: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(folder) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
