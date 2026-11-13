//! A store's file as one engine opens it once (#1886): its writes on a
//! writer, an actor of `tasks` that keeps its connection, and its reads on
//! the writer's owner's blocking threads, on one connection they keep
//! beside the writer's, so a read never waits behind a write. A call that
//! fails lets its connection go, and the next opens the file again.

#![expect(
    clippy::disallowed_types,
    reason = "the reads' connection is taken only inside a blocking read, with nothing else under it and never across an `.await`, so there is no lock order for loom to test (ADR 0060)"
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::tasks::{ActorStopped, StateActor, TaskOwner};

/// A store's file, as [`KeptStore`] opens it.
pub(crate) trait StoreFile: std::fmt::Debug + Send + Sync + 'static {
    /// What its calls fail with.
    type Error: Send + 'static;

    /// Where it lies.
    fn path(&self) -> &Path;

    /// A connection for its writes: the file and its tables made when they
    /// are not there.
    ///
    /// # Errors
    ///
    /// When it cannot be opened, or is of a version this engine refuses.
    fn open(&self) -> Result<Connection, Self::Error>;

    /// A connection for its reads, which take no write lock.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    fn open_read(&self) -> Result<Connection, Self::Error>;

    /// The error of a write asked once the writer stopped with its owner.
    fn stopped(&self, stopped: &ActorStopped) -> Self::Error;
}

/// The connection the writer or the reads keep, opened by their first call;
/// a call that fails lets it go.
type Kept = Option<Connection>;

/// A store's file as one engine opens it: its writer and the connection
/// its reads keep.
#[derive(Debug)]
pub(crate) struct KeptStore<F> {
    file: Arc<F>,
    writer: StateActor<Kept>,
    /// Taken by one read at a time, on a blocking thread, never across an
    /// `.await`.
    reader: Arc<Mutex<Kept>>,
}

impl<F: StoreFile> KeptStore<F> {
    /// `file`, its writer the actor `writer` of `owner` and its reads on
    /// `owner`'s blocking threads; neither opens it before its first call.
    pub(crate) fn of(file: F, writer: &'static str, owner: Arc<TaskOwner>) -> Self {
        Self {
            file: Arc::new(file),
            writer: StateActor::of(writer, owner),
            reader: Arc::default(),
        }
    }

    /// Its file.
    pub(crate) fn file(&self) -> &F {
        &self.file
    }

    /// Runs `work` on the reads' connection, on a blocking thread of the
    /// writer's owner, the connection opened now if none is kept, and
    /// answers what it returned; with a `missing` answer, that one while
    /// the file is not there, which is then not created.
    ///
    /// # Errors
    ///
    /// `work`'s, or the file's when it cannot be opened.
    pub(crate) async fn read<T, W>(&self, missing: Option<T>, work: W) -> Result<T, F::Error>
    where
        T: Send + 'static,
        W: FnOnce(&F, &Connection) -> Result<T, F::Error> + Send + 'static,
    {
        let (file, reader) = (Arc::clone(&self.file), Arc::clone(&self.reader));
        self.writer
            .owner()
            .blocking(move || {
                // A read that panicked may have left its connection anywhere:
                // the next opens the file again.
                let mut kept = reader.lock().unwrap_or_else(|poisoned| {
                    let mut kept = poisoned.into_inner();
                    *kept = None;
                    kept
                });
                match missing {
                    Some(missing) if kept.is_none() && !file.path().exists() => Ok(missing),
                    _ => on(&*file, &mut kept, F::open_read, work),
                }
            })
            .await
    }

    /// Runs `work` on the writer's connection, the file opened, and
    /// created, now if it keeps none, and answers what it returned; with a
    /// `missing` answer, that one while the file is not there, which is
    /// then not created.
    ///
    /// # Errors
    ///
    /// `work`'s, the file's when it cannot be opened, and
    /// [`StoreFile::stopped`]'s once the writer stopped.
    pub(crate) async fn write<T, W>(&self, missing: Option<T>, work: W) -> Result<T, F::Error>
    where
        T: Send + 'static,
        W: FnOnce(&F, &Connection) -> Result<T, F::Error> + Send + 'static,
    {
        let file = Arc::clone(&self.file);
        let answer = self
            .writer
            .run(move |kept| match missing {
                Some(missing) if kept.is_none() && !file.path().exists() => Ok(missing),
                _ => on(&*file, kept, F::open, work),
            })
            .await;
        answer.map_err(|stopped| self.file.stopped(&stopped))?
    }
}

/// Runs `work` on the connection `kept` holds, opened with `open` now if it
/// holds none; a call that fails lets the connection go.
fn on<F: StoreFile, T>(
    file: &F,
    kept: &mut Kept,
    open: fn(&F) -> Result<Connection, F::Error>,
    work: impl FnOnce(&F, &Connection) -> Result<T, F::Error>,
) -> Result<T, F::Error> {
    let connection = match kept {
        Some(connection) => connection,
        None => kept.insert(open(file)?),
    };
    let answer = work(file, connection);
    if answer.is_err() {
        *kept = None;
    }
    answer
}
