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
        match connection.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(())) {
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

#[cfg(test)]
pub(crate) mod tests;
