//! Run-state batches on the run file's writer (ADR 0142): a sealed batch is
//! rows that memory already decided, which the writer writes as one job of
//! one transaction, all or nothing, through a row writer that answers no
//! row, so nothing it writes is decided by a read of the file. One sealed
//! batch waits at a time: the next is queued once the writer committed the
//! one before, which holds back whoever seals them.
//!
//! The rows are generic: each owner of a table writes its own through
//! [`BatchRows`], so the engine names no table of a contribution (ADR 0087).

use std::fmt;
use std::sync::Arc;

use rusqlite::{CachedStatement, Connection, Params};
use tokio::sync::Semaphore;

use super::{Job, Queued, RunState, RunStateError, Syncing, queued_holding};

/// What a batch's rows are written through: statements that change rows
/// and answer none.
#[derive(Clone, Copy)]
pub struct BatchWriter<'connection> {
    connection: &'connection Connection,
}

impl fmt::Debug for BatchWriter<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BatchWriter")
            .finish_non_exhaustive()
    }
}

impl<'connection> BatchWriter<'connection> {
    /// A row writer over `connection`, inside the transaction its caller
    /// holds: the writer's own for a sealed batch, and a write's for rows it
    /// decided itself.
    pub fn over(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    /// `statement`, prepared once per connection, to write rows with.
    ///
    /// # Errors
    ///
    /// SQLite's, when the statement does not prepare.
    pub fn statement(&self, statement: &str) -> rusqlite::Result<BatchStatement<'connection>> {
        self.connection
            .prepare_cached(statement)
            .map(BatchStatement)
    }
}

/// A statement of a [`BatchWriter`], which writes one row at a time.
pub struct BatchStatement<'connection>(CachedStatement<'connection>);

impl fmt::Debug for BatchStatement<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BatchStatement")
            .finish_non_exhaustive()
    }
}

impl BatchStatement<'_> {
    /// Writes `row` with the statement, and answers nothing of it.
    ///
    /// # Errors
    ///
    /// SQLite's, a statement that answers rows included.
    pub fn write<P: Params>(&mut self, row: P) -> rusqlite::Result<()> {
        self.0.execute(row).map(drop)
    }
}

/// Rows of a Run-state batch: values decided before the batch was sealed,
/// which their owner writes into its own tables.
pub trait BatchRows: Send + 'static {
    /// Writes the rows through `writer`.
    ///
    /// # Errors
    ///
    /// SQLite's, which leaves none of the batch written.
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()>;
}

/// How a sealed batch's rows are written, whatever their type.
type WriteRows = dyn FnOnce(&BatchWriter<'_>) -> rusqlite::Result<()> + Send;

/// A sealed Run-state batch: rows nothing is added to any more, ready for
/// the run file's writer.
pub struct SealedBatch(Box<WriteRows>);

impl fmt::Debug for SealedBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedBatch")
            .finish_non_exhaustive()
    }
}

impl SealedBatch {
    /// The sealed batch of `rows`.
    pub fn of<R: BatchRows>(rows: R) -> Self {
        Self(Box::new(move |writer| rows.write(writer)))
    }

    /// Writes its rows through `writer`.
    ///
    /// # Errors
    ///
    /// As [`BatchRows::write`].
    pub fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        (self.0)(writer)
    }
}

/// What a run file's handles share of its batches: the one place a sealed
/// batch waits in.
#[derive(Debug)]
pub(super) struct Batches {
    slot: Arc<Semaphore>,
}

impl Batches {
    pub(super) fn new() -> Self {
        Self {
            slot: Arc::new(Semaphore::new(1)),
        }
    }
}

impl RunState {
    /// Writes `batch` as one job of the writer, in one transaction, all or
    /// nothing, its commit synced as `syncing` says, and answers once that
    /// commit is acknowledged. While the writer has not committed a batch
    /// queued before, this one waits to be queued.
    ///
    /// # Errors
    ///
    /// As [`RunState::write`]; an error of a row leaves none of the batch
    /// written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future while the batch waits to be queued drops the
    /// batch; once queued, it may still commit.
    pub async fn write_batch(
        &self,
        syncing: Syncing,
        batch: SealedBatch,
    ) -> Result<(), RunStateError> {
        let slot = Arc::clone(&self.batches.slot)
            .acquire_owned()
            .await
            .map_err(|_| RunStateError::Closed)?;
        let (write, answered) = queued_holding(
            move |connection| batch.write(&BatchWriter::over(connection)),
            slot,
        );
        self.mailbox
            .send(Queued {
                write,
                job: Job::Write,
                syncing,
            })
            .await
            .map_err(|_| RunStateError::Closed)?;
        answered.await.map_err(|_| RunStateError::Closed)?
    }
}
