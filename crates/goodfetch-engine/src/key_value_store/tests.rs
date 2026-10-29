//! The store's operations on a run file's connection, through the module's
//! interface (spec #213's Seam 4): keys read a page at a time come back
//! once each, in the order of their Unicode scalar values, only those with
//! the prefix; a key of the store's shape is accepted; a value comes back
//! as it was set, nested nulls included.

use std::collections::BTreeSet;

use goodfetch_test_support::{CrashVfs, TestEnv};
use proptest::prelude::*;
use rusqlite::{Connection, OpenFlags};
use serde_json::json;

use super::{Stored, TABLES, get, key_refusal, keys, set};

fn store() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES
        .statements
        .iter()
        .chain(crate::run_files::TABLES.statements)
    {
        connection.execute(statement, []).unwrap();
    }
    connection
}

/// Every key read a page at a time, from the first.
fn every_key(connection: &Connection, prefix: &str, page: usize) -> Vec<String> {
    let mut read: Vec<String> = Vec::new();
    loop {
        let next = keys(connection, prefix, read.last().map(String::as_str), page).unwrap();
        let last = next.len() < page;
        read.extend(next);
        if last {
            return read;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Covers: story:6-31
    #[test]
    fn keys_read_in_pages_come_once_each_in_scalar_order_with_their_prefix(
        stored in proptest::collection::btree_set("[a-c\u{e9}\u{1F600}/]{1,6}", 0..40),
        prefix in "[a-c\u{e9}/]{0,2}",
        page in 1_usize..8,
    ) {
        let connection = store();
        for key in &stored {
            set(&connection, key, &json!(1)).unwrap();
        }

        let read = every_key(&connection, &prefix, page);

        let expected: Vec<String> = stored
            .iter()
            .filter(|key| key.starts_with(prefix.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        prop_assert_eq!(read, expected);
    }

    /// Covers: story:6-34
    #[test]
    fn a_key_of_1_to_512_scalar_values_without_a_control_character_is_accepted(
        key in "[^\\p{Cc}]{1,512}",
    ) {
        prop_assert_eq!(key_refusal(&key), None);
    }
}

/// Covers: story:6-28
#[test]
fn a_value_comes_back_as_it_was_set_with_its_nested_nulls() {
    let connection = store();
    let value = json!({ "cursor": [null, { "page": 2.5 }], "done": false });

    set(&connection, "state", &value).unwrap();

    assert_eq!(
        get(&connection, "state").unwrap(),
        Some(Stored::Json(value))
    );
}

/// A store of 50,000 keys under `k/` and three under `a/`, before them, on
/// the crash VFS with a page cache of 10 pages, so that each page SQLite
/// reads is an I/O operation the VFS counts.
fn counted_store(env: &TestEnv) -> (CrashVfs, Connection) {
    let vfs = CrashVfs::register(&env.unique_name("keys-pages"), env.seed()).unwrap();
    let connection = Connection::open_with_flags_and_vfs(
        env.folder().join("store.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        vfs.name(),
    )
    .unwrap();
    for statement in TABLES
        .statements
        .iter()
        .chain(crate::run_files::TABLES.statements)
    {
        connection.execute(statement, []).unwrap();
    }
    connection.execute_batch("BEGIN").unwrap();
    for key in ["a/1", "a/2", "a/3"] {
        set(&connection, key, &json!(1)).unwrap();
    }
    for number in 0..50_000 {
        set(&connection, &format!("k/{number:06}"), &json!(1)).unwrap();
    }
    connection
        .execute_batch("COMMIT; PRAGMA cache_size = 10;")
        .unwrap();
    (vfs, connection)
}

/// How many I/O operations of `vfs` reading one page of keys took.
fn operations_of(
    vfs: &CrashVfs,
    connection: &Connection,
    prefix: &str,
    after: Option<&str>,
) -> u64 {
    let before = vfs.operations();
    keys(connection, prefix, after, 256).unwrap();
    vfs.operations() - before
}

/// A page of keys reads no more of the store than the first page does,
/// wherever it starts and wherever its prefix's keys end: it searches the
/// key index from its first key and stops past the prefix, so iterating
/// every key takes linear time rather than a scan of the index per page.
///
/// Covers: story:6-31
#[rstest::rstest]
#[case::late_in_a_long_prefix("k/", Some("k/049000"))]
#[case::of_a_prefix_before_many_keys("a/", None)]
fn a_page_of_keys_reads_no_more_of_the_store_than_the_first(
    #[case] prefix: &str,
    #[case] after: Option<&str>,
) {
    let env = TestEnv::new();
    let (vfs, connection) = counted_store(&env);
    let first = operations_of(&vfs, &connection, "k/", None);

    let page = operations_of(&vfs, &connection, prefix, after);

    assert!(
        page <= first,
        "the first page took {first} I/O operations, this one {page}"
    );
}
