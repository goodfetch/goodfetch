use rusqlite::{Connection, OpenFlags};

use super::CountingVfs;

/// A connection in WAL mode, at `synchronous`, on a real file through `vfs`,
/// its table made.
fn connection(vfs: &CountingVfs, folder: &std::path::Path, synchronous: &str) -> Connection {
    let connection = Connection::open_with_flags_and_vfs(
        folder.join("counted.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    connection
        .query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
        .unwrap();
    connection
        .pragma_update(None, "synchronous", synchronous)
        .unwrap();
    connection
        .execute_batch("CREATE TABLE items (value INTEGER)")
        .unwrap();
    connection
}

#[test]
fn a_commit_syncs_once_at_full_and_not_at_normal_on_a_real_file() {
    let (at_full, at_normal) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (full, normal) = (CountingVfs::register(), CountingVfs::register());
    let insert = "INSERT INTO items VALUES (1)";

    let writer = connection(&full, at_full.path(), "FULL");
    let before = full.syncs();
    writer.execute(insert, []).unwrap();
    let at_full_syncs = full.syncs() - before;

    let writer = connection(&normal, at_normal.path(), "NORMAL");
    let before = normal.syncs();
    writer.execute(insert, []).unwrap();
    let at_normal_syncs = normal.syncs() - before;

    assert_eq!((at_full_syncs, at_normal_syncs), (1, 0));
    assert!(at_full.path().join("counted.sqlite-wal").is_file());
}

#[test]
fn each_counting_vfs_counts_its_own_files_only() {
    let folder = tempfile::tempdir().unwrap();
    let (used, other) = (CountingVfs::register(), CountingVfs::register());

    let _connection = connection(&used, folder.path(), "FULL");

    assert_ne!(used.name(), other.name());
    assert!(used.syncs() > 0);
    assert_eq!(other.syncs(), 0);
}

/// The files whose syncs it counts are named, the journal's and the WAL's
/// beside the database's, so a sync of one of them seen from outside the process can be
/// held to its count (ADR 0140's `strace` cross-check).
#[test]
fn it_names_each_file_it_opened_once() {
    let folder = tempfile::tempdir().unwrap();
    let (used, other) = (CountingVfs::register(), CountingVfs::register());

    let _connection = connection(&used, folder.path(), "FULL");
    let _reader = Connection::open_with_flags_and_vfs(
        folder.path().join("counted.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        used.name(),
    )
    .unwrap();

    let folder = folder.path().canonicalize().unwrap();
    assert_eq!(
        used.files(),
        vec![
            folder.join("counted.sqlite"),
            // Its rollback journal, before it turns to WAL.
            folder.join("counted.sqlite-journal"),
            folder.join("counted.sqlite-wal"),
        ]
    );
    assert_eq!(other.files(), Vec::<std::path::PathBuf>::new());
}

/// Each connection that opens a database through it counts one open of
/// that database, however often its file was opened before; its journal's
/// and WAL's opens count none.
#[test]
fn it_counts_each_open_of_a_database() {
    let folder = tempfile::tempdir().unwrap();
    let (used, other) = (CountingVfs::register(), CountingVfs::register());

    let writer = connection(&used, folder.path(), "FULL");
    writer.execute("INSERT INTO items VALUES (1)", []).unwrap();
    for _ in 0..2 {
        let reader = Connection::open_with_flags_and_vfs(
            folder.path().join("counted.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            used.name(),
        )
        .unwrap();
        reader
            .query_row("SELECT count(*) FROM items", [], |_| Ok(()))
            .unwrap();
    }

    assert_eq!((used.opens(), other.opens()), (3, 0));
}
