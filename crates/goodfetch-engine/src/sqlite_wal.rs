//! WAL mode for a SQLite file that several engines may open at once.
//!
//! Switching a new file to WAL takes its exclusive lock, and when another
//! connection holds a shared lock SQLite answers `SQLITE_BUSY` at once
//! rather than wait on the busy timeout, which would risk a deadlock. So
//! engines opening a new file at once wait here for each other's switch,
//! up to the file's own busy timeout. Its calls block: callers run them
//! under their owner's blocking policy.

use std::time::Duration;

use rusqlite::Connection;

/// How long a switch refused as busy waits before it asks again.
const PAUSE: Duration = Duration::from_millis(5);

/// Puts the file `connection` opened in WAL mode, waiting up to `timeout`
/// while another connection switches it.
///
/// # Errors
///
/// SQLite's error, `SQLITE_BUSY` once `timeout` passed.
pub(crate) fn enter(connection: &Connection, timeout: Duration) -> rusqlite::Result<()> {
    let mut waited = Duration::ZERO;
    loop {
        #[expect(
            clippy::disallowed_methods,
            reason = "this module is where a file switches to WAL, waiting while it is busy"
        )]
        let switched = connection.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()));
        match switched {
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

/// Has `connection` leave the write-ahead log as it is when it closes,
/// rather than copy it into the file, which syncs both, when it is the
/// file's last connection (#2110). A commit at `synchronous=FULL` already
/// synced the log, so what it holds survives a power cut, and the next
/// connection reads it; SQLite's automatic checkpoint still copies it into
/// the file once it passes its thousand pages, so it does not grow without
/// bound.
///
/// # Errors
///
/// SQLite's error.
pub(crate) fn no_checkpoint_on_close(connection: &Connection) -> rusqlite::Result<()> {
    connection
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .map(drop)
}

#[cfg(test)]
pub(crate) mod tests;
