//! The store's operations on a run file's connection, through the module's
//! interface (spec #213's Seam 4): keys read a page at a time come back
//! once each, in the order of their Unicode scalar values, only those with
//! the prefix; a key of the store's shape is accepted; a value comes back
//! as it was set, nested nulls included.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use proptest::prelude::*;
use rusqlite::Connection;
use serde_json::json;

use super::{TABLES, get, key_refusal, keys, set};

fn store() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
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

    assert_eq!(get(&connection, "state").unwrap(), Some(value));
}
