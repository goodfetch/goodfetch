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
        self.run(
            true,
            work,
            None::<fn(&Connection, &mut T) -> rusqlite::Result<u64>>,
            None::<fn(&std::path::Path, &mut T) -> std::io::Result<()>>,
        )
        .await
        .map(|(value, _)| value)
    }

    /// Runs `work` on the Run's state, `checkpoint_work` on its kept
    /// Checkpoint (`checkpoint.sqlite`, if present), and `folder_work` on its
    /// run folder, all under the Run's writer lock with `secure_delete` on,
    /// `VACUUM`, and verified WAL truncation, forgetting kept Exports, and
    /// answers both results (ADR 0016).
    pub(super) async fn edit_with_checkpoint<T, F, C, D>(
        self,
        work: F,
        checkpoint_work: C,
        folder_work: D,
    ) -> Result<(T, u64), RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        C: FnOnce(&Connection, &mut T) -> rusqlite::Result<u64> + Send + 'static,
        D: FnOnce(&std::path::Path, &mut T) -> std::io::Result<()> + Send + 'static,
    {
        self.run(true, work, Some(checkpoint_work), Some(folder_work))
            .await
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
            None::<fn(&Connection, &mut T) -> rusqlite::Result<u64>>,
            None::<fn(&std::path::Path, &mut T) -> std::io::Result<()>>,
        )
        .await
        .map(|(value, _)| value)
    }

    async fn run<T, F, C, D>(
        mut self,
        forget_exports: bool,
        work: F,
        checkpoint_work: Option<C>,
        folder_work: Option<D>,
    ) -> Result<(T, u64), RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        C: FnOnce(&Connection, &mut T) -> rusqlite::Result<u64> + Send + 'static,
        D: FnOnce(&std::path::Path, &mut T) -> std::io::Result<()> + Send + 'static,
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
        let mut written = written.map_err(|error| self.of_state(error))?;
        self.truncate_with_retry(&file, self.run_state.vfs.clone())
            .await?;
        if forget_exports {
            let (at, vfs) = (file.clone(), self.run_state.vfs.clone());
            let truncated = self
                .owner
                .blocking(move || vacuum_file(&at, vfs.as_deref()))
                .await
                .map_err(|error| self.unavailable(&error))?;
            if !truncated {
                self.truncate_with_retry(&file, self.run_state.vfs.clone())
                    .await?;
            }
        }
        let checkpoint_count = if let Some(checkpoint_work) = checkpoint_work {
            let (updated, count) = self.run_checkpoint(written, checkpoint_work).await?;
            written = updated;
            count
        } else {
            0
        };
        let at = self.folder.clone();
        let written = self
            .owner
            .blocking(move || {
                let outcome = (|| -> std::io::Result<T> {
                    if let Some(folder_work) = folder_work {
                        folder_work(&at, &mut written)?;
                    }
                    if forget_exports {
                        run_files::forget_exports(&at)?;
                    }
                    Ok(written)
                })();
                drop(lock);
                outcome
            })
            .await
            .map_err(|error| self.unavailable(&error))?;
        Ok((written, checkpoint_count))
    }

    async fn run_checkpoint<T, C>(
        &mut self,
        mut written: T,
        checkpoint_work: C,
    ) -> Result<(T, u64), RunError>
    where
        T: Send + 'static,
        C: FnOnce(&Connection, &mut T) -> rusqlite::Result<u64> + Send + 'static,
    {
        let checkpoint = self.folder.join(CHECKPOINT_FILE);
        let checkpoint_at = checkpoint.clone();
        let (updated, count, truncated) = self
            .owner
            .blocking(move || {
                let (count, truncated) = edit_checkpoint_file(&checkpoint_at, |connection| {
                    checkpoint_work(connection, &mut written)
                })?;
                Ok::<_, rusqlite::Error>((written, count, truncated))
            })
            .await
            .map_err(|error| self.unavailable(&error))?;
        if !truncated {
            self.truncate_with_retry(&checkpoint, None).await?;
        }
        Ok((updated, count))
    }

    async fn truncate_with_retry(
        &mut self,
        file: &std::path::Path,
        vfs: Option<String>,
    ) -> Result<(), RunError> {
        let truncate = || {
            let (at, vfs) = (file.to_path_buf(), vfs.clone());
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
        Ok(())
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

/// Runs `PRAGMA wal_checkpoint(TRUNCATE)` without waiting and answers whether
/// the checkpoint completed with `busy == 0` (matching `discard::truncate_wal`).
fn checkpoint_truncate(connection: &Connection) -> rusqlite::Result<bool> {
    connection.busy_timeout(std::time::Duration::ZERO)?;
    let busy: i64 = connection
        .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")?
        .query_row([], |row| row.get(0))?;
    Ok(busy == 0)
}

/// Vacuums `file` with `secure_delete` on and truncates its WAL, so no
/// residual deleted bytes stay in free pages or WAL frames (ADR 0016).
/// Answers whether the WAL truncation completed with `busy == 0`.
fn vacuum_file(file: &std::path::Path, vfs: Option<&str>) -> rusqlite::Result<bool> {
    let Some(connection) = File::new(file, vfs).connect_existing(Access::ReadWrite)? else {
        return Ok(true);
    };
    connection.pragma_update(None, "secure_delete", "ON")?;
    connection.execute_batch("VACUUM")?;
    checkpoint_truncate(&connection)
}

/// Runs `work` on the kept Checkpoint SQLite file at `checkpoint`, if it
/// exists, with `secure_delete` on and `VACUUM` so no residual deleted bytes
/// stay in the file. Answers `(count, wal_truncated)`.
fn edit_checkpoint_file<C>(checkpoint: &std::path::Path, work: C) -> rusqlite::Result<(u64, bool)>
where
    C: FnOnce(&Connection) -> rusqlite::Result<u64>,
{
    if !checkpoint.is_file() {
        return Ok((0, true));
    }
    let Some(connection) = File::new(checkpoint, None).connect_existing(Access::ReadWrite)? else {
        return Ok((0, true));
    };
    connection.pragma_update(None, "secure_delete", "ON")?;
    let count = sqlite_store::immediate(&connection, work)?;
    connection.execute_batch("VACUUM")?;
    let truncated = checkpoint_truncate(&connection)?;
    Ok((count, truncated))
}
