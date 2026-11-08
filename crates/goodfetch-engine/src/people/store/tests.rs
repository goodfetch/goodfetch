//! The accounts store's file, through its interface.

use goodfetch_test_support::{TestEnv, wal_of};

use super::{AccountsStore, SignInRow};

/// The store holds to the synced per-user store's durability: its WAL stays
/// beside it as a write's connection closes (#2110).
#[test]
fn the_wal_stays_beside_the_store_once_a_write_closed_its_connection() {
    let env = TestEnv::in_memory();
    let store = AccountsStore::at(env.home().join("accounts-store.sqlite"));

    store
        .insert(&SignInRow {
            id: "a-sign-in".to_owned(),
            verifier: "a-verifier".to_owned(),
            person: "ada".to_owned(),
            created: "2026-01-05T09:00:00Z".to_owned(),
            last_used: "2026-01-05T09:00:00Z".to_owned(),
            wrapping_key: "a-wrapping-key".to_owned(),
        })
        .unwrap();

    assert!(wal_of(store.path()).exists());
}
