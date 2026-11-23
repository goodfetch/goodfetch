//! A sheet's rows: an XLSX Export of more Records than a sheet holds rows,
//! too many to write in the Quick suite's budget.
#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_dataset::{ExportOptions, TABLES, WriteExportError, write_export};
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use rusqlite::Connection;
use serde_json::{Value, json};

/// A run file in memory whose Dataset `pages` holds `count` empty Records.
fn run_file(count: i64) -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    let transaction = connection.transaction().unwrap();
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO records VALUES ('pages', ?1, ?1, 'https://shop.test/', '2026-01-05T09:00:00Z', '{}', NULL, '{}')",
            )
            .unwrap();
        for number in 1..=count {
            insert.execute([number]).unwrap();
        }
    }
    transaction.commit().unwrap();
    connection
}

/// The Export of `pages` in `format`.
fn export_of(connection: &Connection, format: ExportFormat) -> Result<Vec<u8>, WriteExportError> {
    write_export(
        connection,
        "pages",
        format,
        ExportOptions::of("pages"),
        ExportCompression::None,
    )
}

/// A sheet holds 1,048,576 rows, its header row included: an XLSX Export
/// of 1,048,575 Records is written, and one of a Record more is refused
/// with `sheet_capacity_exceeded`, naming the rows it would hold and the
/// most a sheet holds; that Dataset exports as CSV.
///
/// Covers: error:sheet_capacity_exceeded, format:export
#[test]
fn an_xlsx_export_of_more_records_than_a_sheet_holds_rows_is_refused() {
    let at = run_file(1_048_575);
    let past = run_file(1_048_576);

    let fitted = export_of(&at, ExportFormat::Xlsx);
    let refused = export_of(&past, ExportFormat::Xlsx);
    let csv = export_of(&past, ExportFormat::Csv);

    assert!(fitted.is_ok(), "{:?}", fitted.err());
    let Err(WriteExportError::Refused(refused)) = refused else {
        panic!("not refused: {refused:?}");
    };
    assert_eq!(refused.code(), "sheet_capacity_exceeded");
    assert_eq!(
        Value::Object(refused.details().unwrap().clone()),
        json!({
            "dataset": "pages",
            "capacity": "rows",
            "figure": 1_048_577,
            "maximum": 1_048_576,
            "fix": "Export `pages` as `csv`, `jsonl` or `parquet`, which hold any number of rows, columns and characters.",
        })
    );
    assert!(csv.is_ok(), "{:?}", csv.err());
}
