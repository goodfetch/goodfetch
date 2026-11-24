//! The Datasets' benchmark scenarios (ADR 0071), shared by the Callgrind
//! and DHAT targets: a Dataset's commit of a thousand Records, one per
//! attempt, into a run file in memory, and the writing of their Export as
//! JSON and as JSONL from that file, through the Datasets' own commit and
//! `run.export`'s pipeline, from a literal pattern, with no I/O, no sleep
//! and no timing.

use goodfetch_dataset::{Committing, Datasets, ExportOptions, TABLES, commit, write_export};
use goodfetch_engine::mask::Masks;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use goodfetch_protocol::point_in_time;
use rusqlite::Connection;
use serde_json::{Value, json};

/// `count` Records of a book shop, each with a title holding accents.
pub(crate) fn records(count: usize) -> Vec<Value> {
    (0..count)
        .map(|n| {
            json!({
                "title": format!("Le Café de la Gare, tome {n}"),
                "price": 12.5 + f64::from(u32::try_from(n % 100).unwrap_or(0)),
                "in_stock": n % 3 != 0,
                "url": format!("https://shop.test/books/{n}"),
                "tags": ["roman", "poche"],
            })
        })
        .collect()
}

/// A run file in memory holding the Datasets' tables and `books`
/// declared, with its declarations.
fn declared() -> (Connection, Datasets) {
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
    (connection, datasets)
}

/// Checks and commits each of `values` into `books` as one attempt's
/// Record, and answers the run file.
pub(crate) fn commit_all(values: Vec<Value>) -> Connection {
    let (connection, datasets) = declared();
    let at = point_in_time::from_rfc3339("2026-01-05T09:00:00Z")
        .expect("invariant: the scenarios' time is RFC 3339");
    let masks = Masks::new();
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
        let id = request.to_string();
        let provenance = RecordProvenance {
            request_id: id.clone(),
            tier: None,
            encoding: None,
            origin: None,
            clean_up_rules: None,
            wall: None,
            payment: None,
            certificate_exception: None,
            robots_txt: None,
            value_transforms: Vec::new(),
            unread_fields: Vec::new(),
        };
        commit(
            &connection,
            Committing {
                request_id: &id,
                url: "https://shop.test/books",
                at: &at,
                provenance: &provenance,
                masks: &masks,
                work: &goodfetch_counted_work::CountedWork::default(),
            },
            &checked,
        )
        .expect("invariant: each Record commits");
    }
    connection
}

/// Writes the Export of `books` in `connection` in `format`, as
/// `run.export` writes it, uncompressed, and answers its size in bytes.
pub(crate) fn export(connection: &Connection, format: ExportFormat) -> usize {
    write_export(
        connection,
        "books",
        format,
        ExportOptions::of("books"),
        ExportCompression::None,
    )
    .expect("invariant: the Records read back")
    .len()
}
