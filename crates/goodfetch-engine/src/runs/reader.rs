//! A reader of one Run's committed state (ADRs 0031 and 0102): it reads the
//! run file read-only, so it never takes the writer lock and never holds
//! the writer back, whichever engine wrote the file and whether or not one
//! still does. While this engine's writer works the Run, it reads on the
//! connections that writer's readers keep (ADR 0138); otherwise on a
//! connection of its own, opened for each read and closed after it, which
//! never outlives the writer of another engine that may still close the
//! file, so that writer stays the last connection and folds its WAL into
//! the file. While this engine's writer works the Run, the reader waits
//! for its next commit or the Run's end; while another engine's writer
//! holds the Run's writer lock, as an Attached engine crawling a Run a
//! reader on the Daemon follows, the reader looks at the run file again
//! every [`ELSEWHERE_POLL`]; a Run with no live writer anywhere has nothing
//! more to come (ADRs 0102 and 0103).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use tokio::sync::watch;

use super::folder::open_reader;
use super::{Ended, RUN_FILE, RunError, Writer, test_writer};
use crate::clock::Clock;
use crate::run_state::RunStateReaders;
use crate::tasks::TaskOwner;

/// How often a reader looks at the run file again while another engine's
/// writer works the Run, whose commits it is not told of.
pub(super) const ELSEWHERE_POLL: Duration = Duration::from_secs(1);

/// A reader of a Run's committed state.
#[derive(Debug)]
pub struct RunReader {
    run_id: RunId,
    folder: PathBuf,
    vfs: Option<String>,
    tasks: Arc<TaskOwner>,
    following: Option<Following>,
    /// Whether this engine's writer worked the Run when the reader opened,
    /// so its writer lock, while held, is this engine's own.
    ours: bool,
    /// Whether another engine's writer held the Run's writer lock at the
    /// last [`RunReader::is_live`].
    elsewhere: bool,
    clock: Clock,
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
    /// The writer's readers, which read the run file beside it.
    pub(super) readers: Option<Arc<RunStateReaders>>,
}

impl RunReader {
    pub(super) fn new(
        run_id: RunId,
        folder: PathBuf,
        vfs: Option<String>,
        tasks: Arc<TaskOwner>,
        following: Option<Following>,
        clock: Clock,
    ) -> Self {
        Self {
            run_id,
            folder,
            vfs,
            tasks,
            ours: following.is_some(),
            following,
            elsewhere: false,
            clock,
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
    /// Run, or, for a Run this engine did not work when the reader opened,
    /// another engine's writer holds its writer lock, which the reader
    /// tests without taking it. Read it before a read, so that read sees
    /// every commit made before the answer turned `false`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn is_live(&mut self) -> bool {
        if let Some(following) = &self.following
            && following.report.borrow().is_some()
        {
            self.following = None;
        }
        if self.following.is_some() {
            return true;
        }
        if self.ours {
            return false;
        }
        let folder = self.folder.clone();
        self.elsewhere = self
            .tasks
            .blocking(move || test_writer(&folder))
            .await
            .is_ok_and(|writer| writer == Writer::Live);
        self.elsewhere
    }

    /// Runs `work` on a read-only connection of the run file's committed
    /// state, on the writer's readers while it follows the Run, otherwise
    /// on one opened for it, under the engine's blocking policy. A
    /// commit made after it started makes the next [`RunReader::changed`]
    /// return.
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
        let readers = self.following.as_mut().and_then(|following| {
            if let Some(commits) = &mut following.commits {
                commits.borrow_and_update();
            }
            following.readers.clone()
        });
        let read = match readers {
            Some(readers) => readers.read(work).await.map(Some),
            None => self.read_opened(work).await,
        };
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

    /// Runs `work` as [`RunReader::read`] does, on a connection opened for
    /// it alone: `None` when the Run has no run file.
    async fn read_opened<T, F>(&self, work: F) -> rusqlite::Result<Option<T>>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let file = self.folder.join(RUN_FILE);
        let vfs = self.vfs.clone();
        self.tasks
            .blocking(move || -> rusqlite::Result<Option<T>> {
                match open_reader(&file, vfs.as_deref())? {
                    Some(connection) => work(&connection).map(Some),
                    None => Ok(None),
                }
            })
            .await
    }

    /// Waits for a commit after the last read began, or for the Run's end;
    /// while another engine's writer works the Run, for [`ELSEWHERE_POLL`];
    /// returns at once when nothing more comes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn changed(&mut self) {
        let Some(Following {
            commits, report, ..
        }) = &mut self.following
        else {
            if self.elsewhere {
                self.clock.sleep(ELSEWHERE_POLL).await;
            }
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
