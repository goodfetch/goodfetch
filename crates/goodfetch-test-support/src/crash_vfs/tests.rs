use rstest::rstest;
use rusqlite::{Connection, ErrorCode, OpenFlags};

use super::{
    AfterCrashError, Change, Crash, CrashKind, CrashVfs, CrashVfsError, File, IoFault, Landed,
    after_crash,
};
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

#[test]
fn a_reader_sees_what_another_connection_commits_to_a_wal_database() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();
    let writer = wal_database(&vfs);
    let reader = open(&vfs);
    let reading = reader.unchecked_transaction().unwrap();
    assert_eq!(values(&reading), Vec::<String>::new());

    insert(&writer, &["committed while a reader reads"]);

    reading.commit().unwrap();
    assert_eq!(values(&reader), ["committed while a reader reads"]);
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
const TORN_WAL_SEED: u64 = 0;

#[test]
fn a_torn_write_leaves_a_wal_database_passing_its_integrity_check() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), TORN_WAL_SEED).unwrap();
    let crashed = wal_database(&vfs);
    insert(&crashed, &["one", "two", "three", "four"]);

    let crash = vfs.crash(CrashKind::PowerCut);

    // Torn mid-write: some but not all of one write's bytes landed, which
    // a VFS that only drops whole writes never gives.
    assert!(
        matches!(
            landed(&crash, "run.sqlite-wal"),
            Some(Landed::Torn { bytes, .. }) if *bytes > 0
        ),
        "the seed tears a WAL write mid-way: {crash:?}"
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

/// A seed whose power cut lands part of a journal-less database's writes
/// (found by trying seeds; a seed that drops them all leaves an empty, whole
/// database).
const DAMAGING_SEED: u64 = 3;

#[test]
fn after_crash_fails_a_database_written_without_a_journal_through_a_power_cut() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), DAMAGING_SEED).unwrap();
    let crashed = open(&vfs);
    crashed
        .execute_batch(
            "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
             CREATE TABLE item (value TEXT); CREATE INDEX item_value ON item (value);
             WITH RECURSIVE counter(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < 2000)
             INSERT INTO item SELECT printf('value %d', n) FROM counter;",
        )
        .unwrap();
    vfs.crash(CrashKind::PowerCut);

    let checked = after_crash(&vfs, DATABASE);

    assert!(
        matches!(checked, Err(AfterCrashError::Damaged { .. })),
        "{checked:?}"
    );
}

#[test]
fn after_crash_fails_when_the_database_is_missing() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();

    let checked = after_crash(&vfs, DATABASE);

    assert!(
        matches!(checked, Err(AfterCrashError::Open(_))),
        "{checked:?}"
    );
}

#[test]
fn a_second_crash_vfs_under_one_name_is_refused() {
    let env = TestEnv::new();
    let name = env.unique_name("crash-vfs");
    let _first = CrashVfs::register(&name, env.seed()).unwrap();

    let second = CrashVfs::register(&name, env.seed());

    assert!(
        matches!(second, Err(CrashVfsError::NameTaken { .. })),
        "{second:?}"
    );
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

#[rstest]
#[case::error(IoFault::Error, ErrorCode::SystemIoFailure)]
#[case::full(IoFault::Full, ErrorCode::DiskFull)]
fn an_injected_fault_reaches_the_caller_as_its_io_error(
    #[case] fault: IoFault,
    #[case] code: ErrorCode,
) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("crash-vfs"), env.seed()).unwrap();
    let connection = open(&vfs);
    connection
        .execute_batch("CREATE TABLE item (value TEXT);")
        .unwrap();
    vfs.fail_at(vfs.operations(), fault);

    let write = connection.execute("INSERT INTO item VALUES ('refused')", []);

    assert_eq!(write.as_ref().map_err(error_code), Err(Some(code)));
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

/// `Debug` names the VFS and reads nothing behind it: neither the disk's
/// bytes nor the shared memory SQLite may be writing through its address.
#[test]
fn a_vfs_debugs_as_its_name_alone() {
    let env = TestEnv::new();
    let name = env.unique_name("crash-vfs");
    let vfs = CrashVfs::register(&name, 1).unwrap();
    let connection = wal_database(&vfs);
    insert(&connection, &["one"]);

    let shown = format!("{vfs:?}");

    assert_eq!(shown, format!("CrashVfs {{ name: {name:?}, .. }}"));
}

/// A file whose durable bytes are `abc` and whose unsynced changes are
/// a write of `XY` at 1, a truncation to 2 bytes and a write of `Z` at 3.
fn file_with_three_changes() -> File {
    File {
        durable: b"abc".to_vec(),
        current: b"aXY Z".to_vec(),
        unsynced: vec![
            Change::Write {
                offset: 1,
                data: b"XY".to_vec(),
            },
            Change::Truncate { size: 2 },
            Change::Write {
                offset: 3,
                data: b"Z".to_vec(),
            },
        ],
    }
}

#[rstest]
#[case::none(Landed::None, b"abc".as_slice())]
#[case::reordered(Landed::Reordered { changes: vec![2, 0] }, b"aXYZ".as_slice())]
#[case::torn_at_a_truncation(Landed::Torn { whole: 1, bytes: 0 }, b"aXY".as_slice())]
#[case::torn_mid_write(Landed::Torn { whole: 0, bytes: 1 }, b"aXc".as_slice())]
fn a_power_cut_lands_on_the_durable_bytes_what_it_drew(
    #[case] landed: Landed,
    #[case] expected: &[u8],
) {
    let mut file = file_with_three_changes();

    file.land(&landed);

    assert_eq!(file.durable, expected);
    assert_eq!(file.current, expected);
}
