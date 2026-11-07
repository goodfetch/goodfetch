//! WAL mode for a SQLite file that several engines may open at once.
//!
//! Switching a new file to WAL takes its exclusive lock, and when another
//! connection holds a shared lock SQLite answers `SQLITE_BUSY` at once
//! rather than wait on the busy timeout, which would risk a deadlock. So
//! engines opening a new file at once wait here for each other's switch,
//! up to the file's own busy timeout. Its calls block: callers run them
//! under their owner's blocking policy. Only `sqlite_store`, which opens
//! every store's file, calls it.

use std::time::Duration;

use rusqlite::Connection;

/// How long a switch refused as busy waits before it asks again.
const PAUSE: Duration = Duration::from_millis(5);

/// Puts the file `connection` opened in WAL mode, waiting up to `timeout`
/// while another connection switches it.
///
/// A new file, which holds no page yet, switches at `synchronous=NORMAL`,
/// its connection's own level set again after (#2117): its switch writes
/// its first page through a rollback journal that holds no page to put
/// back, so the journal's second sync, `FULL`'s, guards nothing. The file
/// itself is still synced, as a WAL holds commits only beside a file
/// whose first page is on the disk.
///
/// # Errors
///
/// SQLite's error, `SQLITE_BUSY` once `timeout` passed.
pub(crate) fn enter(connection: &Connection, timeout: Duration) -> rusqlite::Result<()> {
    let mut waited = Duration::ZERO;
    loop {
        match switch(connection) {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::DatabaseBusy && waited < timeout =>
            {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the callers' SQLite calls run under their blocking policy"
                )]
                std::thread::sleep(PAUSE);
                waited += PAUSE;
            }
            other => return other,
        }
    }
}

/// One try at the switch of [`enter`].
fn switch(connection: &Connection) -> rusqlite::Result<()> {
    let pages: i64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let level: Option<i64> = if pages == 0 {
        let level = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        Some(level)
    } else {
        None
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "this module is where a file switches to WAL, waiting while it is busy"
    )]
    let switched = connection.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()));
    // The switch's own error comes first: a restore that also failed must
    // not hide it, nor a busy file that `enter` waits on.
    let restored = level.map_or(Ok(()), |level| {
        connection.pragma_update(None, "synchronous", level)
    });
    switched.and(restored)
}

#[cfg(test)]
pub(crate) mod tests;
