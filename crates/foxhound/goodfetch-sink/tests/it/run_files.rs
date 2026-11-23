//! Run files holding committed Records, written with the tables
//! `goodfetch-dataset` adds to every crawl's run file, and the SQLite
//! destination's rows read back.

use std::path::Path;

use goodfetch_sink::{RunCopy, SqliteSink};
use rusqlite::Connection;
use serde_json::Value;

/// The table every test delivers into.
pub(crate) const TARGET: &str = "records";

/// A run file in memory whose Datasets hold `records`, each a Dataset's
/// name and the values it committed, numbered from 1 in order.
pub(crate) fn run_file(records: &[(&str, Vec<Value>)]) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in goodfetch_dataset::TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let mut insert = connection
        .prepare_cached(
            "INSERT INTO records (dataset, number, work_item, url, committed_at, data, schema_hash, provenance)
             VALUES (?1, ?2, 'request-1', 'https://shop.test/', '2026-01-05T09:00:00Z', ?3, NULL, '{\"request_id\":\"request-1\"}')",
        )
        .unwrap();
    for (dataset, values) in records {
        for (number, value) in (1_i64..).zip(values) {
            insert
                .execute((dataset, number, value.to_string()))
                .unwrap();
        }
    }
    drop(insert);
    connection
}

/// `count` values `{"item": "<prefix><n>"}`, from 1.
pub(crate) fn items(prefix: &str, count: u32) -> Vec<Value> {
    (1..=count)
        .map(|n| serde_json::json!({ "item": format!("{prefix}{n}") }))
        .collect()
}

/// The copy `run_epoch` of the Run `run-1`.
pub(crate) fn copy(run_epoch: u64) -> RunCopy {
    RunCopy {
        run_id: "run-1".to_owned(),
        run_epoch,
    }
}

/// The SQLite Sink on the destination file `destination`, opened anew as
/// a restarted engine opens it.
pub(crate) fn sink(destination: &Path) -> SqliteSink {
    SqliteSink::new(Connection::open(destination).unwrap(), TARGET)
}

/// One row of the destination's table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) delivery_id: String,
    pub(crate) run_id: String,
    pub(crate) run_epoch: u64,
    pub(crate) dataset: String,
    pub(crate) number: u64,
    pub(crate) committed_at: String,
    pub(crate) record: Value,
}

/// The destination's rows, in the order they were first written.
pub(crate) fn rows(destination: &Path) -> Vec<Row> {
    let connection = Connection::open(destination).unwrap();
    let mut select = connection
        .prepare_cached(
            "SELECT _goodfetch_delivery_id, _goodfetch_run_id, _goodfetch_run_epoch,
                    _goodfetch_dataset, _goodfetch_record_number, _goodfetch_committed_at, record
             FROM records ORDER BY rowid",
        )
        .unwrap();
    select
        .query_map((), |row| {
            let record: String = row.get(6)?;
            let run_epoch: i64 = row.get(2)?;
            let number: i64 = row.get(4)?;
            Ok(Row {
                delivery_id: row.get(0)?,
                run_id: row.get(1)?,
                run_epoch: u64::try_from(run_epoch).unwrap(),
                dataset: row.get(3)?,
                number: u64::try_from(number).unwrap(),
                committed_at: row.get(5)?,
                record: serde_json::from_str(&record).unwrap(),
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The text of each row's record, raw as the destination holds it.
pub(crate) fn record_texts(destination: &Path) -> Vec<String> {
    let connection = Connection::open(destination).unwrap();
    let mut select = connection
        .prepare_cached("SELECT record FROM records ORDER BY rowid")
        .unwrap();
    select
        .query_map((), |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Each row's Dataset and number, in the order first written.
pub(crate) fn numbers(destination: &Path) -> Vec<(String, u64)> {
    rows(destination)
        .into_iter()
        .map(|row| (row.dataset, row.number))
        .collect()
}
