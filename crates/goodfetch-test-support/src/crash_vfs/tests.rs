use rusqlite::{Connection, OpenFlags};

use super::{CrashKind, CrashVfs, after_crash};
use crate::env::TestEnv;

const DATABASE: &str = "run.sqlite";

fn open(vfs: &CrashVfs) -> Connection {
    Connection::open_with_flags_and_vfs(
        DATABASE,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        vfs.name(),
    )
    .unwrap()
}

fn values(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT value FROM item ORDER BY rowid")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn a_database_opened_by_the_vfs_name_reads_back_what_it_committed() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();
    let connection = open(&vfs);

    connection
        .execute_batch("CREATE TABLE item (value TEXT); INSERT INTO item VALUES ('first');")
        .unwrap();

    assert_eq!(values(&open(&vfs)), ["first"]);
}

/// A WAL database whose table is synced, then written with no sync.
fn wal_database_with_an_unsynced_row(vfs: &CrashVfs) -> Connection {
    let connection = open(vfs);
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;
             CREATE TABLE item (value TEXT); INSERT INTO item VALUES ('synced');
             PRAGMA synchronous = OFF; INSERT INTO item VALUES ('unsynced');",
        )
        .unwrap();
    connection
}

#[test]
fn an_unsynced_write_survives_a_process_crash() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();
    let _crashed = wal_database_with_an_unsynced_row(&vfs);

    vfs.crash(CrashKind::Process);

    let reopened = after_crash(&vfs, DATABASE).unwrap();
    assert_eq!(values(&reopened), ["synced", "unsynced"]);
}

#[test]
fn an_unsynced_write_is_gone_after_a_power_cut() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();
    let _crashed = wal_database_with_an_unsynced_row(&vfs);

    vfs.crash(CrashKind::PowerCut);

    let reopened = after_crash(&vfs, DATABASE).unwrap();
    assert_eq!(values(&reopened), ["synced"]);
}
