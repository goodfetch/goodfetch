//! The run file's WAL checkpointer (ADR 0138): the writer's connection
//! runs no automatic checkpoint, its WAL hook notes the WAL's pages after
//! each commit, and an actor of the writer's owner, on a connection of its
//! own, runs a `PASSIVE` checkpoint past the threshold. The writer's last
//! write asks for none: its commit synced the WAL, which stays beside the
//! file, so a clean stop syncs neither the WAL nor the file again (#2117).
//!
//! The WAL stays under a bound (#2323). A `PASSIVE` checkpoint copies what
//! no reader's snapshot still needs, and the log starts again only at a
//! write that finds no reader on it, so readers that follow one another, or
//! one that holds a snapshot, let it grow. Past twice the threshold the
//! writer itself, between two commits, tries a `RESTART` checkpoint on its
//! own connection, which waits [`RESTART_WAIT`] for the readers and holds no
//! lock the writer would wait on; when a reader still holds, it tries again
//! each time the WAL grew by the threshold. A log that started again is
//! truncated to the threshold at its next commit (`journal_size_limit`).
//! Each checkpoint a reader blocked counts in
//! `goodfetch.run_state.blocked_checkpoints`, and the WAL's size in
//! `goodfetch.run_state.wal.size`.

use std::cell::Cell;
use std::path::Path;

use goodfetch_protocol::ops::{RUN_STATE_BLOCKED_CHECKPOINTS, RUN_STATE_WAL_SIZE};
use opentelemetry::metrics::Counter;
use rusqlite::Connection;
use tokio::sync::mpsc;

use super::{Durability, RunStateOptions};
use crate::metrics::{Level, Metrics};
use crate::sqlite_store::{Access, File, Synchronous};
use crate::tasks::TaskOwner;

/// The run file's page size, SQLite's default, which ADR 0138 keeps.
const PAGE_SIZE: u64 = 4096;

/// How long the writer's `RESTART` checkpoint waits for the readers of an
/// old snapshot before it gives up until the WAL grew again: the longest
/// the writer stands still for it at each lock it waits on.
const RESTART_WAIT: std::time::Duration = std::time::Duration::from_millis(10);

thread_local! {
    /// The pages the WAL holds after the last commit on this thread of a
    /// writer's connection, which its WAL hook notes and the writer takes
    /// right after the commit, on the same thread.
    static WAL_PAGES: Cell<std::ffi::c_int> = const { Cell::new(0) };
}

/// Notes the pages the WAL holds after a commit of the writer's connection,
/// for the writer to take once the commit returns.
#[expect(
    clippy::unnecessary_wraps,
    reason = "rusqlite's WAL hook takes this signature"
)]
pub(super) fn note_wal_pages(
    _: &rusqlite::hooks::Wal,
    pages: std::ffi::c_int,
) -> rusqlite::Result<()> {
    WAL_PAGES.set(pages);
    Ok(())
}

/// What a checkpoint answered (`PRAGMA wal_checkpoint`'s row): whether it
/// left frames of the WAL uncopied, or the log not started again, because a
/// reader or another checkpoint held it.
fn blocked(connection: &Connection, mode: &str) -> rusqlite::Result<bool> {
    connection.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |row| {
        let (busy, log, copied): (i64, i64, i64) = (row.get(0)?, row.get(1)?, row.get(2)?);
        Ok(busy != 0 || copied < log)
    })
}

/// The writer's side of its checkpointer: an actor of the writer's owner,
/// which runs each checkpoint on a connection of its own, opened with the
/// writer's, and ends once the writer is gone (ADR 0138).
pub(super) struct WalCheckpointer {
    /// Its requests for a `PASSIVE` checkpoint, which never takes the
    /// writer's lock.
    requests: mpsc::Sender<()>,
    /// The WAL's pages past which a commit asks for a `PASSIVE` checkpoint.
    after_pages: std::ffi::c_int,
    /// The WAL's pages from which the writer next tries to start it again.
    restart_at: std::ffi::c_int,
    /// The level the writer's connection syncs at when it does not sync a
    /// checkpoint as the checkpointer's does: `off`'s.
    unsynced: bool,
    /// `goodfetch.run_state.wal.size`, this run file's share.
    wal_size: Level,
    /// `goodfetch.run_state.blocked_checkpoints`.
    blocked: Counter<u64>,
}

impl WalCheckpointer {
    /// Starts the checkpointer of the run file at `path`, opened as
    /// `options` say, on `connection`, as one of `owner`'s actors.
    pub(super) fn start(
        path: &Path,
        options: &RunStateOptions,
        connection: Connection,
        owner: &TaskOwner,
    ) -> std::io::Result<Self> {
        // Room for one request, which the writer sends only into an empty
        // mailbox.
        let (requests, mailbox) = mpsc::channel(1);
        let path = path.to_owned();
        let metrics = options.metrics.clone().unwrap_or_else(Metrics::none);
        let blocked_checkpoints = metrics
            .meter()
            .u64_counter(RUN_STATE_BLOCKED_CHECKPOINTS.name)
            .with_description(RUN_STATE_BLOCKED_CHECKPOINTS.description)
            .with_unit(RUN_STATE_BLOCKED_CHECKPOINTS.unit)
            .build();
        let counted = blocked_checkpoints.clone();
        owner.actor(
            "run-state-wal-checkpointer",
            mailbox,
            move |_: Vec<()>| match blocked(&connection, "PASSIVE") {
                Ok(false) => {}
                Ok(true) => {
                    counted.add(1, &[]);
                    tracing::debug!(path = %path.display(), "a reader's snapshot kept a WAL checkpoint of the run file from copying the whole log");
                }
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "a WAL checkpoint of the run file failed");
                }
            },
        )?;
        let after_pages = options.wal_checkpoint_after.div_ceil(PAGE_SIZE);
        let after_pages = std::ffi::c_int::try_from(after_pages).unwrap_or(std::ffi::c_int::MAX);
        Ok(Self {
            requests,
            after_pages,
            restart_at: after_pages.saturating_mul(2),
            unsynced: options.durability.synchronous() == Synchronous::Off,
            wal_size: metrics.level(&RUN_STATE_WAL_SIZE),
            blocked: blocked_checkpoints,
        })
    }

    /// Takes the WAL's pages after the commit that just returned on this
    /// thread, on the writer's `connection`, and asks for a checkpoint past
    /// the threshold, unless the commit was the writer's `last` write. Past
    /// twice the threshold it first tries to start the WAL again itself.
    pub(super) fn committed(&mut self, connection: &Connection, path: &Path, last: bool) {
        let pages = WAL_PAGES.take();
        let bytes = u64::try_from(pages).unwrap_or_default() * PAGE_SIZE;
        self.wal_size
            .set(usize::try_from(bytes).unwrap_or(usize::MAX));
        if last || pages < self.after_pages {
            return;
        }
        if pages >= self.restart_at {
            match self.restart(connection) {
                Ok(false) => {
                    self.restart_at = self.after_pages.saturating_mul(2);
                    return;
                }
                Ok(true) => {
                    self.blocked.add(1, &[]);
                    tracing::warn!(path = %path.display(), wal_pages = pages, "a reader's snapshot keeps the run file's WAL from starting again");
                }
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "a WAL checkpoint of the run file failed");
                }
            }
            self.restart_at = pages.saturating_add(self.after_pages);
        }
        // A full or closed mailbox: a checkpoint waits already, or the
        // owner's stop ended the checkpointer.
        let _ = self.requests.try_send(());
    }

    /// Tries a `RESTART` checkpoint on the writer's `connection`, between
    /// two of its commits: whether a reader, or the checkpointer's own
    /// checkpoint under way, blocked it. Its pages are synced before the
    /// log starts again, at `off` too (ADR 0138's note of #1927).
    fn restart(&self, connection: &Connection) -> rusqlite::Result<bool> {
        if self.unsynced {
            connection.pragma_update(None, "synchronous", Synchronous::Normal.pragma())?;
        }
        connection.busy_timeout(RESTART_WAIT)?;
        let answered = blocked(connection, "RESTART");
        let waits = connection.busy_timeout(std::time::Duration::ZERO);
        let synchronous = if self.unsynced {
            connection.pragma_update(None, "synchronous", Synchronous::Off.pragma())
        } else {
            Ok(())
        };
        let answered = answered?;
        waits.and(synchronous).map(|()| answered)
    }
}

/// Opens the run file at `path` through `vfs` for its checkpointer, which
/// syncs as `durability` says, never waits on a lock, as a `PASSIVE`
/// checkpoint takes none it would wait on, and copies nothing as it closes.
pub(super) fn open_wal_checkpointer(
    path: &Path,
    vfs: Option<&str>,
    durability: Durability,
) -> rusqlite::Result<Connection> {
    let connection = File::new(path, vfs).connect(Access::ReadWrite)?;
    connection.busy_timeout(std::time::Duration::ZERO)?;
    let synchronous = durability.checkpoint_synchronous();
    super::connection_durability(synchronous, synchronous).apply(&connection)?;
    Ok(connection)
}
