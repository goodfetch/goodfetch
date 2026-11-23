#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{WriteExportError, write_export};
use crate::encoding::ExportOptions;

/// A run file in memory holding `pages`' Records 1 through `count`, each
/// its number and a title with accents.
fn run_file(count: u32) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in crate::TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    for number in 1..=count {
        connection
            .execute(
                "INSERT INTO records VALUES ('pages', ?1, ?1, 'https://shop.test/', '2026-01-05T09:00:00Z', ?2, NULL, '{}')",
                rusqlite::params![
                    number,
                    format!("{{\"n\":{number},\"title\":\"Le Café, tome {number}\"}}")
                ],
            )
            .unwrap();
    }
    connection
}

/// An Export of more Records than a page holds, compressed page after
/// page, gives back the bytes of the same Export uncompressed.
///
/// Covers: story:6-276
#[rstest]
#[case::gzip(ExportCompression::Gzip)]
#[case::zstd(ExportCompression::Zstd)]
fn a_compressed_export_of_several_pages_gives_back_its_uncompressed_bytes(
    #[case] compression: ExportCompression,
) {
    let connection = run_file(600);
    let export = |compression| {
        write_export(
            &connection,
            "pages",
            ExportFormat::Json,
            ExportOptions::of("pages"),
            compression,
        )
        .unwrap()
    };

    let bytes = export(compression);

    let mut plain = Vec::new();
    if compression == ExportCompression::Gzip {
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut plain)
            .unwrap();
    } else {
        plain = zstd::decode_all(&bytes[..]).unwrap();
    }
    assert_eq!(plain, export(ExportCompression::None));
}

/// A run file in memory whose Dataset `pages` holds `values`, numbered
/// from 1.
fn run_file_of(values: Vec<String>) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in crate::TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    for (number, value) in (1_i64..).zip(values) {
        connection
            .execute(
                "INSERT INTO records VALUES ('pages', ?1, ?1, 'https://shop.test/', '2026-01-05T09:00:00Z', ?2, NULL, '{}')",
                rusqlite::params![number, value],
            )
            .unwrap();
    }
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

/// One Record of `count` fields.
fn fields(count: usize) -> Vec<String> {
    let record: serde_json::Map<String, Value> = (0..count)
        .map(|field| (format!("f{field}"), json!(field)))
        .collect();
    vec![Value::Object(record).to_string()]
}

/// One Record whose field `t` holds `text`.
fn holding(text: &str) -> Vec<String> {
    vec![json!({ "t": text }).to_string()]
}

/// An XLSX Export whose sheet would pass what Excel opens, in columns or
/// one cell's length counted in UTF-16 code units, is refused with
/// `sheet_capacity_exceeded`, as `run.export` refuses it, naming the
/// capacity, the Dataset's figure and the most a sheet holds; the same
/// Dataset one short of it is written, and the refused one exports as CSV.
/// The rows' capacity is `write_export`'s test in `tests/it`, as a run file
/// of a million Records outlasts the Quick suite.
///
/// Covers: error:sheet_capacity_exceeded, format:export
#[rstest]
#[case::columns(fields(16_384), fields(16_385), "columns", 16_385, 16_384)]
#[case::a_string(
    holding(&"a".repeat(32_767)),
    holding(&"a".repeat(32_768)),
    "cell_length",
    32_768,
    32_767
)]
#[case::an_arrays_json_text(
    vec![json!({ "t": ["a".repeat(32_763)] }).to_string()],
    vec![json!({ "t": ["a".repeat(32_764)] }).to_string()],
    "cell_length",
    32_768,
    32_767
)]
#[case::characters_past_the_basic_plane(
    holding(&format!("{}a", "\u{1f600}".repeat(16_383))),
    holding(&"\u{1f600}".repeat(16_384)),
    "cell_length",
    32_768,
    32_767
)]
#[case::a_columns_name(
    vec![json!({ "a".repeat(32_767): 1 }).to_string()],
    vec![json!({ "a".repeat(32_768): 1 }).to_string()],
    "cell_length",
    32_768,
    32_767
)]
fn an_xlsx_sheet_past_what_excel_opens_is_refused(
    #[case] at: Vec<String>,
    #[case] past: Vec<String>,
    #[case] capacity: &str,
    #[case] figure: u64,
    #[case] maximum: u64,
) {
    let at = run_file_of(at);
    let past = run_file_of(past);

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
            "capacity": capacity,
            "figure": figure,
            "maximum": maximum,
            "fix": "Export `pages` as `csv`, `jsonl` or `parquet`, which hold any number of rows, columns and characters.",
        })
    );
    assert!(csv.is_ok(), "{:?}", csv.err());
}
