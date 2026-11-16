//! A Run's discard (ADRs 0001, 0002, 0015 and 0036): what erases the Run's
//! own state, under its writer lock, so that nothing of it is left behind.
//! Its wrapped data key is overwritten in its run file first, with
//! `secure_delete` on and the run file's WAL truncated; in the per-install
//! layout the install database then deletes the run folder's files; the
//! run folder is removed last, with its run file, page bodies, browser
//! profiles and Checkpoints, the lock file after every other entry, since
//! Windows removes no file a handle holds. Its blocking steps run under
//! its owner's blocking policy.
//!
//! The Run's Run index row is the registry's to mark erased. A Profile
//! directory a Run uses in place and a Borrowed browser are outside the run
//! folder, the person's own, and never touched (ADR 0056). No Response
//! cache, compiled Clean-up rules or `filter_lists.extra` list is kept
//! outside a run folder yet (ADRs 0046 and 0047): the ticket that keeps one
//! deletes the discarded Run's entries here.

use std::io;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension as _};

use super::RUN_FILE;
use super::writer::{LOCK_FILE, retried, try_lock};
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::run_state::install::InstallDatabase;
use crate::run_state::{Durability, RunStateError, sqlite_error};
use crate::sqlite_store::{self, Access, File};
use crate::tasks::TaskOwner;

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
    /// Another engine holds its writer lock, or a reader kept its run
    /// file's WAL from being truncated, through every retry.
    #[error("another engine holds the Run's writer lock, or a reader its run file")]
    Held,
    /// Its run file or the install database could not be written.
    #[error(transparent)]
    State(#[from] RunStateError),
    /// The OS refused the run folder or one of its files.
    #[error("the run folder could not be removed: {0}")]
    Io(String),
}

/// What a discard needs beside the Run: its owner, under whose blocking
/// policy it runs, and the clock and entropy its retries wait on.
pub(super) struct Discarding<'a> {
    pub(super) owner: &'a TaskOwner,
    pub(super) clock: &'a Clock,
    pub(super) entropy: &'a mut Entropy,
}

/// Erases the state of the Run `run_id` in `folder`, its run file opening
/// through the VFS `vfs`, and from `install` in the per-install layout. A
/// held writer lock and a WAL a reader keeps are each retried as every
/// writer retries a held lock (ADR 0031), since a reader holds either only
/// for a moment.
pub(super) async fn discard(
    discarding: Discarding<'_>,
    run_id: &str,
    folder: &Path,
    vfs: Option<&str>,
    install: Option<&Arc<InstallDatabase>>,
) -> Result<Discarded, DiscardError> {
    let Discarding {
        owner,
        clock,
        entropy,
    } = discarding;
    let (id, at, held) = (run_id.to_owned(), folder.to_owned(), install.cloned());
    let found = owner
        .blocking(move || present(&id, &at, held.as_deref()))
        .await?;
    let Some(installed) = found else {
        return Ok(Discarded::Absent);
    };
    let lock = match try_lock(owner, folder).await {
        Ok(Some(lock)) => Ok(Some(lock)),
        Ok(None) => retried(clock, entropy, || try_lock(owner, folder)).await,
        Err(error) => Err(error),
    };
    let lock = lock.map_err(DiscardError::Io)?.ok_or(DiscardError::Held)?;
    let file = folder.join(RUN_FILE);
    let (at, opened_with) = (file.clone(), vfs.map(str::to_owned));
    let keyed = owner
        .blocking(move || destroy_key(&at, opened_with.as_deref(), installed))
        .await?;
    if keyed {
        let truncate = || {
            let (at, opened_with) = (file.clone(), vfs.map(str::to_owned));
            owner.blocking(move || truncate_wal(&at, opened_with.as_deref()))
        };
        let truncated = match truncate().await? {
            Some(()) => Some(()),
            None => retried(clock, entropy, truncate).await?,
        };
        truncated.ok_or(DiscardError::Held)?;
    }
    let (id, at, held) = (run_id.to_owned(), folder.to_owned(), install.cloned());
    owner
        .blocking(move || {
            if let Some(install) = held {
                install.erase(&id)?;
            }
            remove_all_but_the_lock(&at).map_err(|error| DiscardError::Io(error.to_string()))?;
            drop(lock);
            remove_folder(&at).map_err(|error| DiscardError::Io(error.to_string()))
        })
        .await?;
    Ok(Discarded::Erased)
}

/// Discards the state of the Run `run_id` in `folder` when its run file,
/// opening through the VFS `vfs`, is one a crash left at `off` (ADR 0138),
/// the Run's Durability level now being `durability`: it then erases what
/// [`discard`] does, under the Run's writer lock, so another engine's live
/// Run, whose file holds the mark too, is refused as held. Its data key is
/// overwritten first when the file can still be written, as a discard
/// does. The run folder and its lock stay, for the Run that starts again.
/// Answers whether it discarded it.
pub(super) async fn discard_unclean(
    discarding: Discarding<'_>,
    run_id: &str,
    folder: &Path,
    vfs: Option<&str>,
    install: Option<&Arc<InstallDatabase>>,
    durability: Durability,
) -> Result<bool, DiscardError> {
    let Discarding {
        owner,
        clock,
        entropy,
    } = discarding;
    let file = folder.join(RUN_FILE);
    let checked = |file: std::path::PathBuf, vfs: Option<String>| {
        owner.blocking(move || crate::run_state::unclean(&file, vfs.as_deref(), durability))
    };
    if !checked(file.clone(), vfs.map(str::to_owned)).await? {
        return Ok(false);
    }
    let lock = match try_lock(owner, folder).await {
        Ok(Some(lock)) => Ok(Some(lock)),
        Ok(None) => retried(clock, entropy, || try_lock(owner, folder)).await,
        Err(error) => Err(error),
    };
    let lock = lock.map_err(DiscardError::Io)?.ok_or(DiscardError::Held)?;
    // Read again under the lock: a Run that started meanwhile ended clean.
    if !checked(file.clone(), vfs.map(str::to_owned)).await? {
        return Ok(false);
    }
    let (id, at, opened_with, held) = (
        run_id.to_owned(),
        folder.to_owned(),
        vfs.map(str::to_owned),
        install.cloned(),
    );
    owner
        .blocking(move || {
            let file = at.join(RUN_FILE);
            // A damaged file may refuse it: its bytes go with the file.
            if destroy_key(&file, opened_with.as_deref(), held.is_some()).is_ok() {
                let _ = truncate_wal(&file, opened_with.as_deref());
            }
            if let Some(install) = held {
                install.erase(&id)?;
            }
            remove_all_but_the_lock(&at).map_err(|error| DiscardError::Io(error.to_string()))?;
            drop(lock);
            Ok::<_, DiscardError>(())
        })
        .await?;
    Ok(true)
}

/// Whether anything of the Run `run_id` is in `folder` or `install`: `None`
/// when nothing is, and otherwise whether `install` holds its run file.
fn present(
    run_id: &str,
    folder: &Path,
    install: Option<&InstallDatabase>,
) -> Result<Option<bool>, DiscardError> {
    let folder_exists =
        std::fs::exists(folder).map_err(|error| DiscardError::Io(error.to_string()))?;
    let installed = match install {
        Some(install) => install.runs()?.iter().any(|run| run == run_id),
        None => false,
    };
    Ok((folder_exists || installed).then_some(installed))
}

/// Opens the run file at `file` through the VFS `vfs`: `None` when it never
/// came to be.
fn open_run_file(file: &Path, vfs: Option<&str>) -> Result<Option<Connection>, RunStateError> {
    File::new(file, vfs)
        .connect_existing(Access::ReadWrite)
        .map_err(|error| sqlite_error(file, &error))
}

/// Overwrites the wrapped data key the run file at `file` holds, if any,
/// with `secure_delete` on (ADR 0015): the zeroed page goes to its WAL,
/// which [`truncate_wal`] then copies over the key. Answers whether there
/// is a run file.
fn destroy_key(file: &Path, vfs: Option<&str>, installed: bool) -> Result<bool, RunStateError> {
    let on_disk = std::fs::exists(file).unwrap_or(false);
    if !on_disk && !installed {
        return Ok(false);
    }
    let failed = |error: rusqlite::Error| sqlite_error(file, &error);
    // A run folder whose run file never came to be holds no key.
    let Some(connection) = open_run_file(file, vfs)? else {
        return Ok(false);
    };
    connection
        .pragma_update(None, "secure_delete", "ON")
        .map_err(failed)?;
    let keyed = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'run_key'")
        .and_then(|mut statement| statement.query_row([], |_| Ok(())))
        .optional()
        .map_err(failed)?
        .is_some();
    if keyed {
        sqlite_store::run_cached(&connection, "DELETE FROM run_key").map_err(failed)?;
    }
    Ok(true)
}

/// Copies the WAL of the run file at `file` into it and truncates it, at
/// once: `None` while a reader keeps a snapshot the checkpoint cannot pass,
/// so the run file may still hold the key's bytes.
fn truncate_wal(file: &Path, vfs: Option<&str>) -> Result<Option<()>, RunStateError> {
    let failed = |error: rusqlite::Error| sqlite_error(file, &error);
    let Some(connection) = open_run_file(file, vfs)? else {
        return Ok(Some(()));
    };
    // Each try answers at once; the retries wait on the engine's clock.
    connection
        .busy_timeout(std::time::Duration::ZERO)
        .map_err(failed)?;
    let busy: i64 = connection
        .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")
        .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
        .map_err(failed)?;
    Ok((busy == 0).then_some(()))
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

#[cfg(test)]
mod tests;
