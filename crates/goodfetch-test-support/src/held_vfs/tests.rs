#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140), and a plain thread standing for a blocking read"
)]

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::HeldVfs;

/// A database at `file` through `vfs`, holding one row.
fn made(vfs: &HeldVfs, file: &Path) {
    let connection = Connection::open_with_flags_and_vfs(
        file,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    connection
        .execute_batch("CREATE TABLE items (value INTEGER); INSERT INTO items VALUES (7);")
        .unwrap();
}

/// The row of the database at `file`, read through `vfs` on a new connection.
fn read(vfs: &HeldVfs, file: &Path) -> i64 {
    let connection = Connection::open_with_flags_and_vfs(
        file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    connection
        .query_row("SELECT value FROM items", [], |row| row.get(0))
        .unwrap()
}

/// A read of a file under the held folder waits until the test lets it go,
/// while a read under another folder answers meanwhile; the read after the
/// held one is not held.
#[test]
fn a_held_read_waits_until_let_go_and_no_other_folder_waits() {
    let (held_folder, other_folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let vfs = HeldVfs::register();
    let (held_file, other_file) = (
        held_folder.path().join("held.sqlite"),
        other_folder.path().join("other.sqlite"),
    );
    made(&vfs, &held_file);
    made(&vfs, &other_file);
    let held = vfs.hold_next(held_folder.path());

    let (waited, other, after) = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| read(&vfs, &held_file));
        assert!(crate::runtime().block_on(held.reached()));
        let other = read(&vfs, &other_file);
        let unfinished = !waiting.is_finished();
        held.let_go();
        let waited = waiting.join().unwrap();
        ((unfinished, waited), other, read(&vfs, &held_file))
    });

    assert_eq!((waited, other, after), ((true, 7), 7, 7));
}

/// A write of a file under the held folder waits too, until let go, and
/// is written then.
#[test]
fn a_held_write_waits_until_let_go() {
    let folder = tempfile::tempdir().unwrap();
    let vfs = HeldVfs::register();
    let file = folder.path().join("held.sqlite");
    made(&vfs, &file);
    let connection = Connection::open_with_flags_and_vfs(
        &file,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    connection
        .query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
        .unwrap();
    let held = vfs.hold_next(folder.path());

    let unfinished = std::thread::scope(|scope| {
        let writing = scope.spawn(move || {
            connection
                .execute("UPDATE items SET value = 8", [])
                .unwrap()
        });
        assert!(crate::runtime().block_on(held.reached()));
        let unfinished = !writing.is_finished();
        held.let_go();
        writing.join().unwrap();
        unfinished
    });

    assert_eq!((unfinished, read(&vfs, &file)), (true, 8));
}

/// Dropping the hold lets its read go, so a failed test leaves no thread
/// waiting.
#[test]
fn a_dropped_hold_lets_its_read_go() {
    let folder = tempfile::tempdir().unwrap();
    let vfs = HeldVfs::register();
    let file = folder.path().join("held.sqlite");
    made(&vfs, &file);
    let held = vfs.hold_next(folder.path());

    let read = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| read(&vfs, &file));
        assert!(crate::runtime().block_on(held.reached()));
        drop(held);
        waiting.join().unwrap()
    });

    assert_eq!(read, 7);
}
