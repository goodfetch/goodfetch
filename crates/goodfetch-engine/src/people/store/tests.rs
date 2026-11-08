//! The accounts store's file, through its own opening.

use goodfetch_test_support::TestEnv;

use super::AccountsStore;

/// The store's commits sync as every synced per-user store's do: each through
/// `F_FULLFSYNC` on macOS, and its WAL stays beside it as its last
/// connection closes (#2110).
#[test]
fn each_commit_syncs_through_fullfsync_on_macos_and_the_wal_stays_as_it_closes() {
    let env = TestEnv::in_memory();
    let store = AccountsStore::at(env.home().join("accounts-store.sqlite"));

    let connection = store.open().unwrap();
    let fullfsync: i64 = connection
        .pragma_query_value(None, "fullfsync", |row| row.get(0))
        .unwrap();
    drop(connection);

    assert_eq!(fullfsync, i64::from(cfg!(target_os = "macos")));
    let mut wal = store.path().as_os_str().to_owned();
    wal.push("-wal");
    assert!(std::path::Path::new(&wal).exists());
}
