use rstest::rstest;
use rusqlite::{Connection, ErrorCode, OpenFlags};

use super::{Crash, CrashKind, CrashVfs, Landed, after_crash};
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

/// Commits each value in its own transaction.
fn insert(connection: &Connection, values: &[&str]) {
    for value in values {
        connection
            .execute("INSERT INTO item VALUES (?1)", [value])
            .unwrap();
    }
}

fn landed<'a>(crash: &'a Crash, path: &str) -> Option<&'a Landed> {
    crash
        .files
        .iter()
        .find(|file| file.path == path)
        .map(|file| &file.landed)
}

/// A WAL database whose table is synced, then that syncs only at
/// checkpoints, as `synchronous = NORMAL` does.
fn wal_database(vfs: &CrashVfs) -> Connection {
    let connection = open(vfs);
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;
             CREATE TABLE item (value TEXT); PRAGMA synchronous = NORMAL;",
        )
        .unwrap();
    connection
}

/// A seed whose power cut, after [`wal_database`] and four commits, tears
/// one of the WAL's unsynced writes (found by trying seeds).
const TORN_WAL_SEED: u64 = 3;

#[test]
fn a_torn_write_leaves_a_wal_database_passing_its_integrity_check() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), TORN_WAL_SEED).unwrap();
    let crashed = wal_database(&vfs);
    insert(&crashed, &["one", "two", "three", "four"]);

    let crash = vfs.crash(CrashKind::PowerCut);

    assert!(
        matches!(landed(&crash, "run.sqlite-wal"), Some(Landed::Torn { .. })),
        "the seed tears the WAL: {crash:?}"
    );
    after_crash(&vfs, DATABASE).unwrap();
}

/// The crash and the values left after a power cut drawn from `seed`.
fn power_cut(env: &TestEnv, seed: u64) -> (Crash, Vec<String>) {
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), seed).unwrap();
    let crashed = wal_database(&vfs);
    insert(&crashed, &["one", "two", "three", "four"]);
    let crash = vfs.crash(CrashKind::PowerCut);
    (crash, values(&after_crash(&vfs, DATABASE).unwrap()))
}

#[test]
fn the_same_seed_gives_the_same_power_cut_twice() {
    let env = TestEnv::new();

    let (first, second) = (power_cut(&env, env.seed()), power_cut(&env, env.seed()));

    assert_eq!(first, second);
}

#[test]
fn another_seed_can_give_another_power_cut() {
    let env = TestEnv::new();

    let (first, second) = (power_cut(&env, TORN_WAL_SEED), power_cut(&env, 4));

    assert_ne!(first, second);
}

fn error_code(error: &rusqlite::Error) -> Option<ErrorCode> {
    error.sqlite_error_code()
}

#[rstest]
#[case::process(CrashKind::Process)]
#[case::power_cut(CrashKind::PowerCut)]
fn a_connection_opened_before_a_crash_fails_its_next_io(#[case] kind: CrashKind) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();
    let crashed = wal_database_with_an_unsynced_row(&vfs);
    vfs.crash(kind);

    let write = crashed.execute("INSERT INTO item VALUES ('after the crash')", []);

    assert_eq!(
        write.as_ref().map_err(error_code),
        Err(Some(ErrorCode::SystemIoFailure))
    );
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
