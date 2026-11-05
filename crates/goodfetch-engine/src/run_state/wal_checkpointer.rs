//! The run file's WAL checkpointer (ADR 0138): the writer's connection
//! runs no automatic checkpoint, its WAL hook notes the WAL's pages after
//! each commit, and an actor of the writer's owner, on a connection of its
//! own, runs a `PASSIVE` checkpoint past the threshold and a `TRUNCATE` one
//! once the writer's last write committed.

use std::cell::Cell;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use tokio::sync::mpsc;

use super::{Durability, RunStateOptions};
use crate::tasks::TaskOwner;

/// The run file's page size, SQLite's default, which ADR 0138 keeps.
const PAGE_SIZE: u64 = 4096;

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

/// What the writer asks its checkpointer for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalCheckpoint {
    /// A `PASSIVE` checkpoint, which never takes the writer's lock.
    Passive,
    /// A `TRUNCATE` checkpoint, once the writer's last write committed.
    Truncate,
}

/// The writer's side of its checkpointer: an actor of the writer's owner,
/// which runs each checkpoint on a connection of its own, opened with the
/// writer's, and ends once the writer is gone (ADR 0138).
pub(super) struct WalCheckpointer {
    requests: mpsc::Sender<WalCheckpoint>,
    /// The WAL's pages past which a commit asks for a `PASSIVE` checkpoint.
    after_pages: std::ffi::c_int,
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
        // Room for one `PASSIVE` request, which the writer sends only into an
        // empty mailbox, and its last write's `TRUNCATE` one.
        let (requests, mailbox) = mpsc::channel(2);
        let path = path.to_owned();
        owner.actor(
            "run-state-wal-checkpointer",
            mailbox,
            move |group: Vec<WalCheckpoint>| {
                let mode = if group.contains(&WalCheckpoint::Truncate) {
                    WalCheckpoint::Truncate
                } else {
                    WalCheckpoint::Passive
                };
                if let Err(error) = checkpoint_wal(&connection, mode) {
                    tracing::warn!(path = %path.display(), %error, ?mode, "a WAL checkpoint of the run file failed");
                }
            },
        )?;
        let after_pages = options.wal_checkpoint_after.div_ceil(PAGE_SIZE);
        Ok(Self {
            requests,
            after_pages: std::ffi::c_int::try_from(after_pages).unwrap_or(std::ffi::c_int::MAX),
        })
    }

    /// Takes the WAL's pages after the commit that just returned on this
    /// thread, and asks for a `TRUNCATE` checkpoint after the writer's
    /// `last` write, else a `PASSIVE` one past the threshold, unless one
    /// waits already.
    pub(super) fn committed(&self, last: bool) {
        let pages = WAL_PAGES.take();
        let request = if last {
            WalCheckpoint::Truncate
        } else if pages >= self.after_pages
            && self.requests.capacity() == self.requests.max_capacity()
        {
            WalCheckpoint::Passive
        } else {
            return;
        };
        // A full or closed mailbox: a checkpoint waits already, or the
        // owner's stop ended the checkpointer.
        let _ = self.requests.try_send(request);
    }
}

/// Opens the run file at `path` through `vfs` for its checkpointer, which
/// syncs as `durability` says and never waits on a lock: a `PASSIVE`
/// checkpoint takes none it would wait on, and a `TRUNCATE` one gives up.
pub(super) fn open_wal_checkpointer(
    path: &Path,
    vfs: Option<&str>,
    durability: Durability,
) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = match vfs {
        Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs),
        None => Connection::open_with_flags(path, flags),
    }?;
    connection.busy_timeout(std::time::Duration::ZERO)?;
    connection.pragma_update(None, "synchronous", durability.checkpoint_synchronous())?;
    if cfg!(target_os = "macos") {
        connection.pragma_update(None, "fullfsync", "ON")?;
        connection.pragma_update(None, "checkpoint_fullfsync", "ON")?;
    }
    Ok(connection)
}

/// Runs a checkpoint of `mode` on `connection`.
fn checkpoint_wal(connection: &Connection, mode: WalCheckpoint) -> rusqlite::Result<()> {
    let statement = match mode {
        WalCheckpoint::Passive => "PRAGMA wal_checkpoint(PASSIVE)",
        WalCheckpoint::Truncate => "PRAGMA wal_checkpoint(TRUNCATE)",
    };
    connection.query_row(statement, [], |_| Ok(()))
}
