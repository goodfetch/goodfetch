//! A person's write to the state of a Run no engine runs (ADR 0031), such
//! as the erasing of one of its Datasets (spec #1937, story 214): it takes
//! the Run's writer lock, retried as every writer retries a held lock, then
//! the next run epoch, which fences any writer that held the file before,
//! and writes under it, with `secure_delete` on so that the pages it frees
//! are overwritten (ADR 0015). Its run file's WAL is then copied into it
//! and truncated, and the Run's kept Exports, written from its state
//! before, are deleted, before the lock is let go.

use std::path::PathBuf;

use goodfetch_protocol::run::RunId;
use rusqlite::Connection;

use super::discard::truncate_wal;
use super::folder::read_recorded;
use super::writer::{retried, try_lock};
use super::{RUN_FILE, RunError};
use crate::checkpoint::CHECKPOINT_FILE;
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::run_files;
use crate::run_state::{self, RunState, RunStateOptions, RunStateTables};
use crate::sqlite_store::{self, Access, File};
use crate::tasks::{OwnerKind, TaskOwner};

/// A write to the state of a Run no engine runs, and what it needs.
pub(super) struct Editing {
    pub(super) run_id: RunId,
    pub(super) folder: PathBuf,
    pub(super) run_state: RunStateOptions,
    pub(super) tables: Vec<RunStateTables>,
    pub(super) clock: Clock,
    pub(super) entropy: Entropy,
    /// Under whose blocking policy it runs.
    pub(super) owner: TaskOwner,
}

impl Editing {
    /// Runs `work` in one transaction of the Run's state, under its writer
    /// lock and a run epoch of its own, and answers what `work` answers,
    /// forgetting any kept Exports since `work` may erase Records.
    pub(super) async fn edit<T, F>(self, work: F) -> Result<T, RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.run(true, work, None::<fn(&Connection) -> rusqlite::Result<u64>>)
            .await
            .map(|(value, _)| value)
    }

    /// Runs `work` on the Run's state and `checkpoint_work` on its kept
    /// Checkpoint (`checkpoint.sqlite`, if present), both under the Run's
    /// writer lock with `secure_delete` on and `VACUUM`, forgetting kept
    /// Exports, and answers both results (ADR 0016).
    pub(super) async fn edit_with_checkpoint<T, F, C>(
        self,
        work: F,
        checkpoint_work: C,
    ) -> Result<(T, u64), RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        C: FnOnce(&Connection) -> rusqlite::Result<u64> + Send + 'static,
    {
        self.run(true, work, Some(checkpoint_work)).await
    }

    /// Runs `work` in one transaction of the Run's state, under its writer
    /// lock and a run epoch of its own, keeping the Run's Exports intact:
    /// for post-Run delivery bookkeeping such as removing delivered dead
    /// letters.
    pub(super) async fn update<T, F>(self, work: F) -> Result<T, RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.run(
            false,
            work,
            None::<fn(&Connection) -> rusqlite::Result<u64>>,
        )
        .await
        .map(|(value, _)| value)
    }

    async fn run<T, F, C>(
        mut self,
        forget_exports: bool,
        work: F,
        checkpoint_work: Option<C>,
    ) -> Result<(T, u64), RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        C: FnOnce(&Connection) -> rusqlite::Result<u64> + Send + 'static,
    {
        let file = self.folder.join(RUN_FILE);
        let (at, vfs) = (file.clone(), self.run_state.vfs.clone());
        let recorded = self
            .owner
            .blocking(move || read_recorded(&at, vfs.as_deref()))
            .await
            .map_err(|error| self.unavailable(&error))?;
        if recorded.is_none() {
            return Err(RunError::NotFound(self.run_id));
        }
        let (owner, folder) = (&self.owner, &self.folder);
        let lock = match try_lock(owner, folder).await {
            Ok(Some(lock)) => Ok(Some(lock)),
            Ok(None) => retried(&self.clock, &mut self.entropy, || try_lock(owner, folder)).await,
            Err(error) => Err(error),
        };
        let Some(lock) = lock.map_err(|error| self.unavailable(&error))? else {
            return Err(self.in_use());
        };
        let (options, tables, next) = (self.run_state.clone(), self.tables.clone(), file.clone());
        let epoch = self
            .owner
            .blocking(move || run_state::next_epoch(&next, &options, &tables, 0))
            .await
            .map_err(|error| self.of_state(error))?;
        tracing::info!(
            target: "goodfetch::lease",
            transition = "take_epoch",
            run_id = %self.run_id,
            run_epoch = epoch,
            "took the Run's writer role to write a person's change"
        );
        let writer = self.owner.child_for(OwnerKind::Run);
        let state = RunState::open(
            file.clone(),
            &self.run_state,
            self.tables.clone(),
            epoch,
            &writer,
        )
        .await
        .map_err(|error| self.of_state(error))?;
        let written = state
            .write(move |connection| {
                connection.pragma_update(None, "secure_delete", "ON")?;
                work(connection)
            })
            .await;
        drop(state);
        writer.wait().await;
        let written = written.map_err(|error| self.of_state(error))?;
        let truncate = || {
            let (at, vfs) = (file.clone(), self.run_state.vfs.clone());
            self.owner
                .blocking(move || truncate_wal(&at, vfs.as_deref()))
        };
        let truncated = match truncate().await.map_err(|error| self.of_state(error))? {
            Some(()) => Some(()),
            None => retried(&self.clock, &mut self.entropy, truncate)
                .await
                .map_err(|error| self.of_state(error))?,
        };
        if truncated.is_none() {
            return Err(self.in_use());
        }
        if forget_exports {
            let (at, vfs) = (file.clone(), self.run_state.vfs.clone());
            self.owner
                .blocking(move || vacuum_file(&at, vfs.as_deref()))
                .await
                .map_err(|error| self.unavailable(&error))?;
        }
        let checkpoint_count = if let Some(checkpoint_work) = checkpoint_work {
            let checkpoint = self.folder.join(CHECKPOINT_FILE);
            self.owner
                .blocking(move || edit_checkpoint_file(&checkpoint, checkpoint_work))
                .await
                .map_err(|error| self.unavailable(&error))?
        } else {
            0
        };
        let at = self.folder.clone();
        self.owner
            .blocking(move || {
                let forgotten = if forget_exports {
                    run_files::forget_exports(&at)
                } else {
                    Ok(())
                };
                drop(lock);
                forgotten
            })
            .await
            .map_err(|error| self.unavailable(&error))?;
        Ok((written, checkpoint_count))
    }

    fn unavailable(&self, error: &impl ToString) -> RunError {
        RunError::Unavailable {
            run_id: self.run_id.clone(),
            folder: self.folder.clone(),
            error: error.to_string(),
        }
    }

    fn of_state(&self, error: run_state::RunStateError) -> RunError {
        RunError::of_state(&self.run_id, &self.folder, error)
    }

    fn in_use(&self) -> RunError {
        RunError::InUse {
            run_id: self.run_id.clone(),
            folder: self.folder.clone(),
        }
    }
}

/// Vacuums `file` with `secure_delete` on and truncates its WAL, so no
/// residual deleted bytes stay in free pages or WAL frames (ADR 0016).
fn vacuum_file(file: &std::path::Path, vfs: Option<&str>) -> rusqlite::Result<()> {
    let Some(connection) = File::new(file, vfs).connect_existing(Access::ReadWrite)? else {
        return Ok(());
    };
    connection.pragma_update(None, "secure_delete", "ON")?;
    connection.execute_batch("VACUUM")?;
    let _ = connection
        .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")
        .and_then(|mut statement| statement.query_row([], |row| row.get::<_, i64>(0)));
    Ok(())
}

/// Runs `work` on the kept Checkpoint SQLite file at `checkpoint`, if it
/// exists, with `secure_delete` on and `VACUUM` so no residual deleted bytes
/// stay in the file.
fn edit_checkpoint_file<C>(checkpoint: &std::path::Path, work: C) -> rusqlite::Result<u64>
where
    C: FnOnce(&Connection) -> rusqlite::Result<u64>,
{
    if !checkpoint.is_file() {
        return Ok(0);
    }
    let Some(connection) = File::new(checkpoint, None).connect_existing(Access::ReadWrite)? else {
        return Ok(0);
    };
    connection.pragma_update(None, "secure_delete", "ON")?;
    let count = sqlite_store::immediate(&connection, work)?;
    connection.execute_batch("VACUUM")?;
    let _ = connection
        .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")
        .and_then(|mut statement| statement.query_row([], |row| row.get::<_, i64>(0)));
    Ok(count)
}
