//! Each run's tables as table functions: a query reads only the runs it
//! selected, joins and aggregates them, reads the Records `run.records`
//! reads, and never a secret field.

use goodfetch_protocol::foxhound::sql_tab::QueryRefusal;
use goodfetch_sql_tab::Failure;
use goodfetch_test_support::{Canaries, Output, SecretKind};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::run_files::{LIMITS, book, run_file, sandbox};

/// Covers: story:12-210
#[test]
fn a_query_reads_the_records_run_records_reads() {
    let folder = tempfile::tempdir().expect("a folder");
    let records: Vec<Value> = (1..=3).map(book).collect();
    let path = run_file(folder.path(), "shop", &records, "none");
    let expected = goodfetch_dataset::read(
        &Connection::open(&path).expect("the run file opens"),
        goodfetch_dataset::Query {
            dataset: "books",
            after: 0,
            through: None,
            wall: None,
        },
        100,
    )
    .expect("run.records' reader reads");

    let answer = sandbox(&[("shop", &path)], LIMITS)
        .query(
            "SELECT number, value, request_id, url, committed_at FROM records('shop') ORDER BY number",
            None,
        )
        .expect("the query runs");

    let read: Vec<Value> = answer
        .rows
        .iter()
        .map(|row| {
            json!({
                "number": row[0],
                "value": serde_json::from_str::<Value>(row[1].as_str().expect("JSON text")).expect("JSON"),
                "request_id": row[2],
                "url": row[3],
            })
        })
        .collect();
    let wanted: Vec<Value> = expected
        .iter()
        .map(|record| {
            json!({
                "number": record.number,
                "value": record.value,
                "request_id": record.request_id,
                "url": record.url,
            })
        })
        .collect();
    assert_eq!(read, wanted);
    assert_eq!(answer.total, 3);
    assert_eq!(
        answer
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.column_type.as_str()))
            .collect::<Vec<_>>(),
        [
            ("number", "BIGINT"),
            ("value", "VARCHAR"),
            ("request_id", "VARCHAR"),
            ("url", "VARCHAR"),
            ("committed_at", "VARCHAR"),
        ]
    );
}

/// Covers: story:12-210
#[test]
fn a_query_joins_and_aggregates_the_runs_it_selected() {
    let folder = tempfile::tempdir().expect("a folder");
    let monday = run_file(folder.path(), "monday", &[book(1), book(2)], "none");
    let tuesday = run_file(folder.path(), "tuesday", &[book(2), book(3)], "none");

    let answer = sandbox(&[("monday", &monday), ("tuesday", &tuesday)], LIMITS)
        .query(
            "WITH seen AS (SELECT run, value->>'title' AS title FROM records('monday') \
             UNION ALL SELECT run, value->>'title' FROM records('tuesday')) \
             SELECT title, count(*) AS runs FROM seen GROUP BY title ORDER BY title",
            None,
        )
        .expect("the query runs");

    assert_eq!(
        answer.rows,
        [
            vec![json!("Café 1"), json!(1)],
            vec![json!("Café 2"), json!(2)],
            vec![json!("Café 3"), json!(1)],
        ]
    );
}

/// Covers: story:12-211
#[test]
fn only_the_selected_runs_are_registered() {
    let folder = tempfile::tempdir().expect("a folder");
    let chosen = run_file(folder.path(), "chosen", &[book(1)], "none");
    run_file(folder.path(), "other", &[book(2)], "none");
    let sandbox = sandbox(&[("chosen", &chosen)], LIMITS);

    let refused = sandbox.query("SELECT * FROM records('other')", None);
    let allowed = sandbox.query("SELECT count(*) FROM records('chosen')", None);

    assert!(
        matches!(
            refused,
            Err(Failure::Invalid {
                reason: QueryRefusal::RunNotSelected,
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(
        allowed.expect("the chosen run is read").rows,
        [vec![json!(1)]]
    );
}

/// Covers: story:12-211
#[test]
fn secret_fields_are_never_columns_and_no_canary_is_read() {
    let folder = tempfile::tempdir().expect("a folder");
    let mut canaries = Canaries::new(1208);
    let secret = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let path = run_file(folder.path(), "shop", &[book(1), book(2)], &secret);
    let sandbox = sandbox(&[("shop", &path)], LIMITS);

    let answers: Vec<String> = [
        "SELECT * FROM records('shop')",
        "SELECT * FROM datasets('shop')",
        "SELECT * FROM rejected_records('shop')",
    ]
    .into_iter()
    .map(|sql| {
        let answer = sandbox.query(sql, None).expect("the query runs");
        json!({ "columns": answer.columns.iter().map(|column| &column.name).collect::<Vec<_>>(), "rows": answer.rows })
            .to_string()
    })
    .collect();

    let outputs: Vec<Output<'_>> = answers
        .iter()
        .map(|answer| Output::Bytes {
            name: "answer",
            bytes: answer.as_bytes(),
        })
        .collect();
    assert_eq!(canaries.leaks(&outputs).expect("the search runs"), []);
    assert!(
        answers.iter().all(|answer| !answer.contains("schema_hash")),
        "{answers:?}"
    );
}
