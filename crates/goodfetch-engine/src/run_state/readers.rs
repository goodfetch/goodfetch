//! A run file's readers (ADR 0138): a bounded group of read-only
//! connections, kept open from one read to the next while the file's writer
//! works, on which every read of its committed state runs beside the
//! writer, on its owner's blocking threads. A read never waits for a
//! commit: WAL gives it the file as of the commits before it began, and a
//! write is answered only once its commit is in the file, so a read that
//! follows its own write sees it.
//!
//! The group closes its connections when the writer stops, before the
//! writer's own connection closes: the writer is then the file's last
//! connection, which folds the WAL into the file and removes it, so a Run
//! that ended leaves one self-contained file, as it did before readers were
//! kept. A read after that opens a connection for itself alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;
use tokio::sync::{Mutex, Semaphore};

use super::{STATEMENTS, SWITCH_TIMEOUT, run_cached};
use crate::sqlite_store::{Access, File};
use crate::tasks::BlockingThreads;

/// How many reads of one run file run at once, each on a connection of its
/// own: more wait for one to end.
const READERS: usize = 4;

/// The readers of one run file.
#[derive(Debug)]
pub(crate) struct RunStateReaders {
    path: PathBuf,
    vfs: Option<String>,
    threads: BlockingThreads,
    /// One per connection, held for the length of a read.
    permits: Arc<Semaphore>,
    idle: Mutex<Idle>,
}

/// The connections no read holds now, whether the writer has stopped, and
/// the statements each connection prepares before its next read.
#[derive(Debug, Default)]
struct Idle {
    connections: Vec<Kept>,
    closed: bool,
    prepared: Prepared,
}

/// The statements the Run named for its readers to prepare (ADR 0140), and
/// how many times it named them, which a connection's own count lags until
/// it prepares them.
#[derive(Clone, Debug, Default)]
struct Prepared {
    statements: Arc<Vec<&'static str>>,
    named: u64,
}

/// A connection kept for the next read, with the [`Prepared::named`] count
/// whose statements it prepared.
#[derive(Debug)]
struct Kept {
    connection: Connection,
    prepared: u64,
}

impl RunStateReaders {
    /// The readers of the run file at `path`, opened through the VFS `vfs`,
    /// which read on `threads`. None opens before its first read.
    pub(super) fn new(path: PathBuf, vfs: Option<String>, threads: BlockingThreads) -> Self {
        Self {
            path,
            vfs,
            threads,
            permits: Arc::new(Semaphore::new(READERS)),
            idle: Mutex::new(Idle::default()),
        }
    }

    /// Runs `work` on a read-only connection of the group, opened the first
    /// time the group has none free, and answers what it returned; once the
    /// writer stopped, on a connection opened for it alone.
    ///
    /// # Errors
    ///
    /// SQLite's, when the file cannot be opened, or `work`'s.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting: a read that started runs to
    /// its end, and its connection then closes.
    pub(crate) async fn read<T, F>(&self, work: F) -> rusqlite::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .expect("invariant: a run file's readers' permits are never closed");
        let (kept, prepared) = {
            let mut idle = self.idle.lock().await;
            (idle.connections.pop(), idle.prepared.clone())
        };
        let (path, vfs) = (self.path.clone(), self.vfs.clone());
        let (kept, answer) = self
            .threads
            .run(move || {
                let (connection, named) = match kept {
                    Some(Kept {
                        connection,
                        prepared,
                    }) => (connection, prepared),
                    None => match open(&path, vfs.as_deref()) {
                        Ok(connection) => (connection, 0),
                        Err(error) => return (None, Err(error)),
                    },
                };
                if named != prepared.named {
                    prepare(&connection, &prepared.statements);
                }
                let answer = work(&connection);
                // A connection whose transaction did not end is not reused.
                let kept = connection.is_autocommit().then_some(Kept {
                    connection,
                    prepared: prepared.named,
                });
                (kept, answer)
            })
            .await;
        if let Some(connection) = kept {
            let mut idle = self.idle.lock().await;
            if !idle.closed {
                idle.connections.push(connection);
            }
        }
        drop(permit);
        answer
    }

    /// Has each connection prepare `statements` before its next read, once,
    /// so that no read of a unit's prepares one (ADR 0140): what a Run names
    /// as it starts or resumes, as its writer prepares them.
    pub(super) async fn prepare(&self, statements: Vec<&'static str>) {
        let mut idle = self.idle.lock().await;
        idle.prepared = Prepared {
            statements: Arc::new(statements),
            named: idle.prepared.named + 1,
        };
    }

    /// Closes every connection no read holds, and keeps none from now on:
    /// the writer stops.
    pub(super) fn close(&self) {
        // The lock is held only to take or give back a connection, never
        // across a read, so it frees at once.
        let mut idle = loop {
            match self.idle.try_lock() {
                Ok(idle) => break idle,
                Err(_) => std::thread::yield_now(),
            }
        };
        idle.closed = true;
        idle.connections.clear();
    }
}

/// Opens the run file at `path` read-only, through the VFS `vfs`.
fn open(path: &Path, vfs: Option<&str>) -> rusqlite::Result<Connection> {
    let connection = File::new(path, vfs).connect(Access::ReadOnly)?;
    connection.set_prepared_statement_cache_capacity(STATEMENTS);
    connection.busy_timeout(SWITCH_TIMEOUT)?;
    Ok(connection)
}

/// Prepares each of `statements` on `connection`, into its cache, in no
/// transaction. One that does not prepare is prepared at its first use.
fn prepare(connection: &Connection, statements: &[&'static str]) {
    if let Err(error) = statements
        .iter()
        .try_for_each(|statement| connection.prepare_cached(statement).map(drop))
    {
        tracing::warn!(%error, "a run file's reader did not prepare the statements its Run named");
    }
}

/// Runs `work` in one read transaction, so all its statements see the file
/// as of the same commits.
pub(super) fn in_one_transaction<T>(
    connection: &Connection,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    run_cached(connection, "BEGIN")?;
    match work(connection) {
        Ok(answer) => {
            run_cached(connection, "COMMIT")?;
            Ok(answer)
        }
        Err(error) => {
            if !connection.is_autocommit() {
                let _ = run_cached(connection, "ROLLBACK");
            }
            Err(error)
        }
    }
}
