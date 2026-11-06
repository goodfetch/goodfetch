//! The run file's WAL checkpointer (ADR 0138): the writer's connection
//! runs no automatic checkpoint, its WAL hook notes the WAL's pages after
//! each commit, and an actor of the writer's owner, on a connection of its
//! own, runs a `PASSIVE` checkpoint past the threshold. The writer's last
//! write asks for none: its commit synced the WAL, which stays beside the
//! file, so a clean stop syncs neither the WAL nor the file again (#2117).

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

/// The writer's side of its checkpointer: an actor of the writer's owner,
/// which runs each checkpoint on a connection of its own, opened with the
/// writer's, and ends once the writer is gone (ADR 0138).
pub(super) struct WalCheckpointer {
    /// Its requests for a `PASSIVE` checkpoint, which never takes the
    /// writer's lock.
    requests: mpsc::Sender<()>,
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
        // Room for one request, which the writer sends only into an empty
        // mailbox.
        let (requests, mailbox) = mpsc::channel(1);
        let path = path.to_owned();
        owner.actor(
            "run-state-wal-checkpointer",
            mailbox,
            move |_: Vec<()>| {
                if let Err(error) =
                    connection.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
                {
                    tracing::warn!(path = %path.display(), %error, "a WAL checkpoint of the run file failed");
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
    /// thread, and asks for a checkpoint past the threshold, unless the
    /// commit was the writer's `last` write.
    pub(super) fn committed(&self, last: bool) {
        let pages = WAL_PAGES.take();
        if last || pages < self.after_pages {
            return;
        }
        // A full or closed mailbox: a checkpoint waits already, or the
        // owner's stop ended the checkpointer.
        let _ = self.requests.try_send(());
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
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = match vfs {
        Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs),
        None => Connection::open_with_flags(path, flags),
    }?;
    connection.busy_timeout(std::time::Duration::ZERO)?;
    crate::sqlite_wal::no_checkpoint_on_close(&connection)?;
    connection.pragma_update(None, "synchronous", durability.checkpoint_synchronous())?;
    if cfg!(target_os = "macos") {
        connection.pragma_update(None, "fullfsync", "ON")?;
        connection.pragma_update(None, "checkpoint_fullfsync", "ON")?;
    }
    Ok(connection)
}
