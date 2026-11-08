//! A store's SQLite file as a test reaches it from outside the store: the
//! write-ahead log beside it, and its pages damaged as a failing disk
//! damages them.

use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use rusqlite::Connection;

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
/// # Panics
///
/// When the file cannot be read or written, or holds no such table.
pub fn damage_table(path: &Path, table: &str) {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap();
    let roots: Vec<i64> = connection
        .prepare("SELECT rootpage FROM sqlite_master WHERE tbl_name = ?1 AND rootpage > 0")
        .unwrap()
        .query_map([table], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!roots.is_empty(), "the file holds no table `{table}`");
    drop(connection);
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    for root in roots {
        file.seek(SeekFrom::Start(
            u64::try_from((root - 1) * page_size).unwrap(),
        ))
        .unwrap();
        file.write_all(&vec![0xff; usize::try_from(page_size).unwrap()])
            .unwrap();
    }
}
