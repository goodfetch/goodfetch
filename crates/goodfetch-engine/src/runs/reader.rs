//! A reader of one Run's committed state (ADRs 0031 and 0102): it reads the
//! run file on a read-only connection of its own, opened for each read and
//! closed after it, so it never takes the writer lock and never holds the
//! writer back, whichever engine wrote the file and whether or not one
//! still does. While this engine's writer works the Run, the reader waits
//! for its next commit or the Run's end; a Run with no live writer here has
//! nothing more to come.

use std::path::PathBuf;
use std::sync::Arc;

use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use tokio::sync::watch;

use super::folder::open_reader;
use super::{Ended, RUN_FILE, RunError};
use crate::tasks::TaskOwner;

/// A reader of a Run's committed state.
#[derive(Debug)]
pub struct RunReader {
    run_id: RunId,
    folder: PathBuf,
    vfs: Option<String>,
    tasks: Arc<TaskOwner>,
    following: Option<Following>,
}

/// What opens a Run's state read-only, as a reader that takes no lock.
#[derive(Clone, Debug)]
pub struct RunFileOpener {
    file: PathBuf,
    vfs: Option<String>,
}

impl RunFileOpener {
    /// A read-only connection of the run file's committed state; `None`
    /// when the Run has no run file.
    ///
    /// # Errors
    ///
    /// SQLite's, when the file cannot be opened.
    pub fn open(&self) -> rusqlite::Result<Option<Connection>> {
        open_reader(&self.file, self.vfs.as_deref())
    }
}

/// What a reader follows of a Run this engine works: its writer's commits
/// and its end.
#[derive(Debug)]
pub(super) struct Following {
    /// The writer's commits, or `None` once the Run is ending.
    pub(super) commits: Option<watch::Receiver<u64>>,
    /// The Run's report, once it ended.
    pub(super) report: watch::Receiver<Option<Ended>>,
}

impl RunReader {
    pub(super) fn new(
        run_id: RunId,
        folder: PathBuf,
        vfs: Option<String>,
        tasks: Arc<TaskOwner>,
        following: Option<Following>,
    ) -> Self {
        Self {
            run_id,
            folder,
            vfs,
            tasks,
            following,
        }
    }

    /// The Run's folder.
    pub(super) fn folder(&self) -> &std::path::Path {
        &self.folder
    }

    /// What opens the run file read-only as each of this reader's reads
    /// does, for a reader that reads off the engine's tasks, such as the
    /// SQL tab's table functions, which DuckDB calls on its own threads
    /// (ADR 0057).
    pub fn opener(&self) -> RunFileOpener {
        RunFileOpener {
            file: self.folder.join(RUN_FILE),
            vfs: self.vfs.clone(),
        }
    }

    /// Runs `work` under the engine's blocking policy, opening nothing:
    /// work that reads the run file through [`RunReader::opener`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting: the work runs to its end.
    pub async fn blocking<T, F>(&self, work: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.tasks.blocking(work).await
    }

    /// Whether more may be committed: this engine's writer still works the
    /// Run. Read it before a read, so that read sees every commit made
    /// before the answer turned `false`.
    pub fn is_live(&mut self) -> bool {
        if let Some(following) = &self.following
            && following.report.borrow().is_some()
        {
            self.following = None;
        }
        self.following.is_some()
    }

    /// Runs `work` on a read-only connection of the run file's committed
    /// state, under the engine's blocking policy. A commit made after it
    /// started makes the next [`RunReader::changed`] return.
    ///
    /// # Errors
    ///
    /// `NotFound` when the Run has no run file, `Unavailable` when it
    /// cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting: the read runs to its end.
    pub async fn read<T, F>(&mut self, work: F) -> Result<T, RunError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        if let Some(Following {
            commits: Some(commits),
            ..
        }) = &mut self.following
        {
            commits.borrow_and_update();
        }
        let file = self.folder.join(RUN_FILE);
        let vfs = self.vfs.clone();
        let read = self
            .tasks
            .blocking(move || -> rusqlite::Result<Option<T>> {
                match open_reader(&file, vfs.as_deref())? {
                    Some(connection) => work(&connection).map(Some),
                    None => Ok(None),
                }
            })
            .await;
        match read {
            Ok(Some(answer)) => Ok(answer),
            Ok(None) => Err(RunError::NotFound(self.run_id.clone())),
            Err(error) => Err(RunError::Unavailable {
                run_id: self.run_id.clone(),
                folder: self.folder.clone(),
                error: error.to_string(),
            }),
        }
    }

    /// Waits for a commit after the last read began, or for the Run's end;
    /// returns at once when nothing more comes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn changed(&mut self) {
        let Some(Following { commits, report }) = &mut self.following else {
            return;
        };
        let ended = report.wait_for(Option::is_some);
        let (mut registry_gone, mut writer_gone) = (false, false);
        match commits.as_mut() {
            Some(writer) => {
                tokio::select! {
                    ended = ended => registry_gone = ended.is_err(),
                    changed = writer.changed() => writer_gone = changed.is_err(),
                }
            }
            None => registry_gone = ended.await.is_err(),
        }
        if writer_gone {
            *commits = None;
        }
        if registry_gone {
            self.following = None;
        }
    }
}
