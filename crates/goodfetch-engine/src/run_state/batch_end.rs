//! What a Run writes at the end of each batch the writer commits (#2169):
//! learning's evidence, written behind, is taught once per written batch,
//! off each page's path (ADR 0142's note of 2026-11-10). It runs on the
//! writer's thread, in the batch's transaction, just before its commit,
//! whether the writer commits each group or batches attempts, as a write
//! of its own: the watch of the writes is told as it begins and when it
//! fails, and a failure rolls back its own changes alone, the batch
//! committing without them.

use std::sync::{Arc, OnceLock};

use rusqlite::Connection;

use super::{Failed, RunStateError, Running, commit_error, run_in_savepoint};

/// What a Run writes at the end of each batch, told on the writer's
/// thread.
pub trait BatchEnd: Send + Sync + std::fmt::Debug {
    /// Whether it has anything to write: a batch's commit runs it only
    /// then.
    fn waiting(&self) -> bool;

    /// Writes it in the batch's transaction on `connection`, before its
    /// commit.
    ///
    /// # Errors
    ///
    /// The run state's: its own changes are then rolled back.
    fn ending(&self, connection: &Connection) -> rusqlite::Result<()>;
}

/// Where the writer finds what it writes at the end of each batch, which a
/// handle installs once.
pub(super) type Ending = Arc<OnceLock<Arc<dyn BatchEnd>>>;

/// Runs `ending`, when it waits, in the transaction `running` holds, as a
/// write of its own: answers the error that ended that transaction, when
/// one did, which fails the batch's commit. Any other failure rolls back
/// its own changes only.
pub(super) fn end(running: Running<'_>, ending: Option<&dyn BatchEnd>) -> Option<RunStateError> {
    let ending = ending.filter(|ending| ending.waiting())?;
    let Running {
        connection,
        path,
        watch,
    } = running;
    if let Some(watch) = watch {
        watch.began();
    }
    let Err(Failed { error, ended }) =
        run_in_savepoint(connection, |connection| ending.ending(connection))
    else {
        return None;
    };
    if let Some(watch) = watch {
        watch.failed();
    }
    tracing::warn!(path = %path.display(), %error, "what a batch's end writes was rolled back");
    ended.then(|| commit_error(path, &error))
}
