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
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::run_files;
use crate::run_state::{self, RunState, RunStateOptions, RunStateTables};
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
    /// lock and a run epoch of its own, and answers what `work` answers.
    pub(super) async fn edit<T, F>(mut self, work: F) -> Result<T, RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
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
        let at = self.folder.clone();
        self.owner
            .blocking(move || {
                let forgotten = run_files::forget_exports(&at);
                drop(lock);
                forgotten
            })
            .await
            .map_err(|error| self.unavailable(&error))?;
        Ok(written)
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
