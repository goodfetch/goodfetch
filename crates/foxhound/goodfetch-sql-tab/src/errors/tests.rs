//! One named case per DuckDB error class the translation point knows, and
//! an unknown failure, each raised by a real DuckDB connection (ADR 0127).
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a thread of the test's own interrupts DuckDB's query, outside any async runtime"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use duckdb::{Config, Connection};
use goodfetch_protocol::foxhound::sql_tab::QueryRefusal;
use rstest::rstest;

use super::{Failure, one_read, translate};

/// A locked connection as the sandbox opens it, with `memory` of memory.
fn connection(memory: &str) -> Connection {
    let config = Config::default()
        .enable_external_access(false)
        .expect("external access turns off")
        .enable_autoload_extension(false)
        .expect("autoloading turns off")
        .max_memory(memory)
        .expect("the memory limit is set")
        .with("max_temp_directory_size", "0B")
        .expect("spilling turns off");
    Connection::open_in_memory_with_flags(config).expect("DuckDB opens in memory")
}

/// The failure of running `sql`.
fn failure_of(connection: &Connection, sql: &str) -> Failure {
    let error = connection
        .prepare(sql)
        .and_then(|mut statement| {
            let mut rows = statement.query([])?;
            while rows.next()?.is_some() {}
            Ok(())
        })
        .expect_err("the statement fails");
    translate(&error)
}

fn invalid(reason: QueryRefusal) -> impl Fn(&Failure) -> bool {
    move |failure| matches!(failure, Failure::Invalid { reason: found, .. } if *found == reason)
}

/// Covers: story:12-211
#[rstest]
#[case::parser("SELECT 1 +", invalid(QueryRefusal::Syntax))]
#[case::binder(
    "SELECT missing_column FROM range(3)",
    invalid(QueryRefusal::UnknownName)
)]
#[case::catalog("SELECT * FROM missing_table", invalid(QueryRefusal::UnknownName))]
#[case::permission(
    "SELECT * FROM read_csv('/etc/hostname')",
    invalid(QueryRefusal::OutsideRead)
)]
#[case::out_of_memory(
    "SELECT list(range) FROM range(10000000)",
    |failure: &Failure| *failure == Failure::MemoryExceeded
)]
#[case::unknown("SELECT CAST('not a number' AS INTEGER)", |failure: &Failure| *failure == Failure::Unrecognized)]
fn each_duckdb_error_class_answers_its_code(
    #[case] sql: &str,
    #[case] expected: impl Fn(&Failure) -> bool,
) {
    let failure = failure_of(&connection("16MB"), sql);

    assert!(expected(&failure), "{sql}: {failure:?}");
}

/// Covers: story:12-211
#[test]
fn an_interrupt_is_the_engines_at_the_deadline() {
    let connection = connection("512MB");
    let interrupt = connection.interrupt_handle();
    let done = Arc::new(AtomicBool::new(false));
    let ended = Arc::clone(&done);
    // Interrupted until it ends: an interrupt sent before the query starts
    // reaches nothing.
    let stopper = std::thread::spawn(move || {
        while !ended.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            interrupt.interrupt();
        }
    });

    let failure = failure_of(
        &connection,
        "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3",
    );
    done.store(true, Ordering::SeqCst);
    stopper.join().expect("the interrupting thread ends");

    assert_eq!(failure, Failure::Interrupted);
}

/// Covers: story:12-211
#[rstest]
#[case::insert("INSERT INTO t VALUES (1)", QueryRefusal::NotARead)]
#[case::attach("ATTACH 'other.duckdb'", QueryRefusal::NotARead)]
#[case::two_statements("SELECT 1; SELECT 2", QueryRefusal::NotARead)]
#[case::syntax("SELECT 1 +", QueryRefusal::Syntax)]
fn only_one_select_or_with_is_a_read(#[case] sql: &str, #[case] reason: QueryRefusal) {
    let connection = connection("64MB");
    let serialized: String = connection
        .query_row("SELECT json_serialize_sql(?)", [sql], |row| row.get(0))
        .expect("json_serialize_sql answers");

    let checked = one_read(&serialized);

    assert!(invalid(reason)(&checked.expect_err("refused")), "{sql}");
}

/// Covers: story:12-211
#[rstest]
#[case::select("SELECT 1")]
#[case::with("WITH t AS (SELECT 1 AS a) SELECT a FROM t")]
fn a_select_or_with_is_a_read(#[case] sql: &str) {
    let connection = connection("64MB");
    let serialized: String = connection
        .query_row("SELECT json_serialize_sql(?)", [sql], |row| row.get(0))
        .expect("json_serialize_sql answers");

    assert_eq!(one_read(&serialized), Ok(()));
}
