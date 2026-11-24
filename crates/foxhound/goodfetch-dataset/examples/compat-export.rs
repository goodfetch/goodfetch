//! The Export profile's writer for the Compatibility corpus (ADR 0066):
//! declares `books`, commits three Records through the Datasets' own
//! commit at ADR 0059's test `Clock` start, the shared Unicode record the
//! second and the third read past a paywall, in a run file in memory, then
//! writes their Export in each format through `run.export`'s pipeline,
//! `books.json`, `books.jsonl`, `books.csv`, `books.xlsx`,
//! `books.html`, `books.md`, `books.xml`, `books.parquet`, `books.avro`,
//! `books.arrow`, `books.rss` and `books.atom`, into the folder given as
//! the only argument. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::{Path, PathBuf};

use goodfetch_dataset::{Committing, Datasets, ExportOptions, TABLES, commit, write_export};
use goodfetch_engine::mask::Masks;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_protocol::foxhound::provenance::{RecordProvenance, WallKind, WallMarker};
use goodfetch_protocol::point_in_time;
use goodfetch_test_support::unicode_record;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let committed = committed();
    write(&output, &committed);
}

/// A run file in memory holding the three Records, committed.
fn committed() -> Connection {
    let connection = Connection::open_in_memory().expect("invariant: SQLite opens in memory");
    for statement in TABLES.statements {
        connection
            .execute_batch(statement)
            .expect("invariant: the Datasets' tables are created");
    }
    let datasets = Datasets::declare(&[DatasetDeclaration {
        name: "books".to_owned(),
        schema: None,
        key: Vec::new(),
    }])
    .expect("invariant: one Dataset declares");
    datasets
        .record(&connection)
        .expect("invariant: the declaration is recorded");
    let unicode = Value::Object(
        unicode_record()
            .fields
            .into_iter()
            .map(|field| (field.name, Value::String(field.text)))
            .collect::<Map<_, _>>(),
    );
    let values = [
        json!({
            "title": "Café", "price": 12.5, "in_stock": true, "tags": ["roman", "poche"],
            "url": "https://shop.test/1", "published": "2026-01-02T08:00:00Z"
        }),
        unicode,
        json!({ "title": "Ünïcode", "price": null, "in_stock": false, "tags": [] }),
    ];
    let at = point_in_time::from_rfc3339(START).expect("invariant: the corpus start is RFC 3339");
    for (request, value) in values.into_iter().enumerate() {
        let checked = datasets
            .check(
                vec![PushedRecord {
                    dataset: "books".to_owned(),
                    data: RecordData::of(&value).expect("invariant: a JSON value writes as JSON"),
                }],
                &goodfetch_counted_work::CountedWork::default(),
            )
            .expect("invariant: each Record holds");
        let id = (request + 1).to_string();
        let wall = (request == 2).then_some(WallMarker {
            kind: WallKind::Paywall,
            vendor: None,
        });
        commit(
            &connection,
            Committing {
                request_id: &id,
                url: &format!("https://shop.test/{id}"),
                at: &at,
                provenance: &RecordProvenance {
                    request_id: id.clone(),
                    tier: None,
                    encoding: None,
                    origin: None,
                    clean_up_rules: None,
                    wall,
                    payment: None,
                    certificate_exception: None,
                    robots_txt: None,
                    value_transforms: Vec::new(),
                    unread_fields: Vec::new(),
                },
                masks: &Masks::new(),
                work: &goodfetch_counted_work::CountedWork::default(),
            },
            &checked,
        )
        .expect("invariant: each Record commits");
    }
    connection
}

/// Writes the Export of `books` in the run file `committed` in each
/// format into `output`.
fn write(output: &Path, committed: &Connection) {
    for (format, name) in [
        (ExportFormat::Json, "books.json"),
        (ExportFormat::Jsonl, "books.jsonl"),
        (ExportFormat::Csv, "books.csv"),
        (ExportFormat::Xlsx, "books.xlsx"),
        (ExportFormat::HtmlTable, "books.html"),
        (ExportFormat::Markdown, "books.md"),
        (ExportFormat::Xml, "books.xml"),
        (ExportFormat::Parquet, "books.parquet"),
        (ExportFormat::Avro, "books.avro"),
        (ExportFormat::ArrowIpc, "books.arrow"),
        (ExportFormat::Rss, "books.rss"),
        (ExportFormat::Atom, "books.atom"),
    ] {
        let bytes = write_export(
            committed,
            "books",
            format,
            ExportOptions::of("books"),
            ExportCompression::None,
        )
        .expect("invariant: the Records read back");
        std::fs::write(output.join(name), bytes).expect("invariant: the output folder is writable");
    }
}
