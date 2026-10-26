//! The limits a query runs under: its memory, past which it fails as
//! `query_memory_exceeded`; the engine's interrupt at its deadline; and
//! its row limit, past which rows are counted, not returned.

use std::time::Duration;

use goodfetch_sql_tab::{Failure, Limits};
use serde_json::json;

use crate::run_files::{LIMITS, book, run_file, sandbox};

/// A query that holds far more than 16 MB at once.
const HEAVY: &str = "SELECT list(range) FROM range(10000000)";

/// Covers: story:12-211
#[test]
fn a_query_past_its_memory_limit_is_stopped() {
    let folder = tempfile::tempdir().expect("a folder");
    let path = run_file(folder.path(), "shop", &[book(1)], "none");
    let sandbox = sandbox(
        &[("shop", &path)],
        Limits {
            memory: 16 * 1024 * 1024,
            rows: 10_000,
        },
    );

    let failed = sandbox.query(HEAVY, None);
    let allowed = sandbox.query("SELECT count(*) FROM records('shop')", None);

    assert_eq!(failed, Err(Failure::MemoryExceeded));
    assert_eq!(allowed.expect("the run is read").rows, [vec![json!(1)]]);
}

/// Covers: story:12-211
#[test]
fn an_interrupted_query_stops() {
    let folder = tempfile::tempdir().expect("a folder");
    let path = run_file(folder.path(), "shop", &[book(1)], "none");
    let sandbox = sandbox(&[("shop", &path)], LIMITS);
    let stop = sandbox.stopper();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        stop.stop();
    });

    let failed = sandbox.query(
        "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3",
        None,
    );
    stopper.join().expect("the interrupting thread ends");

    assert_eq!(failed, Err(Failure::Interrupted));
}

/// Covers: story:12-211
#[test]
fn rows_past_the_limit_are_counted_not_returned() {
    let folder = tempfile::tempdir().expect("a folder");
    let records: Vec<_> = (1..=5000).map(book).collect();
    let path = run_file(folder.path(), "shop", &records, "none");
    let sandbox = sandbox(
        &[("shop", &path)],
        Limits {
            memory: LIMITS.memory,
            rows: 3,
        },
    );

    let limited = sandbox
        .query("SELECT number FROM records('shop') ORDER BY number", None)
        .expect("the query runs");
    let asked = sandbox
        .query(
            "SELECT number FROM records('shop') ORDER BY number",
            Some(2),
        )
        .expect("the query runs");

    assert_eq!(
        (limited.rows, limited.total),
        (vec![vec![json!(1)], vec![json!(2)], vec![json!(3)]], 5000)
    );
    assert_eq!(
        (asked.rows, asked.total),
        (vec![vec![json!(1)], vec![json!(2)]], 5000)
    );
}
