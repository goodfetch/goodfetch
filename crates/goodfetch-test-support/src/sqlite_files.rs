//! A store's SQLite file as a test reaches it from outside the store: the
//! write-ahead log beside it, and its pages damaged as a failing disk
//! damages them.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// Why [`damage_table`] damaged nothing.
#[derive(Debug, thiserror::Error)]
pub enum DamageTableError {
    /// SQLite could not read the file.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The file could not be written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The file holds no such table.
    #[error("the file holds no table `{0}`")]
    NoTable(String),
}

/// Where the write-ahead log of the SQLite file at `path` is.
pub fn wal_of(path: &Path) -> PathBuf {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    PathBuf::from(wal)
}

/// Overwrites the pages at the root of the table `table` and of its indexes
/// in the SQLite file at `path`, its WAL first copied into it, with bytes
/// that are no B-tree page: SQLite reads them as it reads a page that met
/// an I/O error, whichever of them a query starts from.
///
/// # Errors
///
/// When the file cannot be read or written, or holds no such table.
#[expect(
    clippy::disallowed_methods,
    reason = "the test damages its own store's file, as a failing disk would, beneath every counted function; its SQL statements are its own, on no Shared path (ADR 0140)"
)]
pub fn damage_table(path: &Path, table: &str) -> Result<(), DamageTableError> {
    let connection = Connection::open(path)?;
    connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    let page_size: u32 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let roots: Vec<u32> = connection
        .prepare("SELECT rootpage FROM sqlite_master WHERE tbl_name = ?1 AND rootpage > 0")?
        .query_map([table], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(connection);
    if roots.is_empty() {
        return Err(DamageTableError::NoTable(table.to_owned()));
    }
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    for root in roots {
        let page_size = u64::from(page_size);
        file.seek(SeekFrom::Start((u64::from(root) - 1) * page_size))?;
        std::io::copy(&mut std::io::repeat(0xff).take(page_size), &mut file)?;
    }
    Ok(())
}
