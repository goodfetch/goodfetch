//! A store's opening under the durability its caller chose.

use std::time::Duration;

use goodfetch_test_support::{TestEnv, wal_of};

use super::{Access, Durability, File, Folder, Opening, Wait};

/// A synced per-user store's commits each sync through `F_FULLFSYNC` on
/// macOS, which no tool can observe reaching the drive, and its WAL stays
/// beside it as its last connection closes (#2110). Each such store has its
/// own test of the WAL, through its own write.
#[test]
fn a_synced_stores_commits_use_fullfsync_on_macos_and_its_wal_stays_as_it_closes() {
    let env = TestEnv::in_memory();
    let path = env.folder().join("store.sqlite");

    let connection = File::new(&path, None)
        .open(&Opening {
            folder: Folder::Exists,
            access: Access::Create,
            wait: Wait::Busy(Duration::from_secs(5)),
            durability: Durability::SYNCED,
            counting: None,
            schema: None,
        })
        .unwrap();
    connection
        .execute_batch("CREATE TABLE written (value INTEGER) STRICT")
        .unwrap();
    let fullfsync: i64 = connection
        .pragma_query_value(None, "fullfsync", |row| row.get(0))
        .unwrap();
    drop(connection);

    assert_eq!(fullfsync, i64::from(cfg!(target_os = "macos")));
    assert!(wal_of(&path).exists());
}
