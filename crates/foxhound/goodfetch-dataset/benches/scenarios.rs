//! The Datasets' benchmark scenarios (ADR 0071), shared by the Callgrind
//! and DHAT targets: a Dataset's commit of a thousand Records, one per
//! attempt, into a run file in memory, and the writing of their Export as
//! JSON and as JSONL from that file, through the Datasets' own commit,
//! read and encoder, from a literal pattern, with no I/O, no sleep and no
//! timing.

use goodfetch_dataset::{
    Columns, Committing, Datasets, ExportEncoder, ExportOptions, Exported, Query, TABLES, commit,
    read_values,
};
use goodfetch_engine::mask::Masks;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::export::ExportFormat;
use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use goodfetch_protocol::run::PointInTime;
use rusqlite::Connection;
use serde_json::{Value, json};

/// The most Records one read takes, as `run.export` reads them.
const PAGE: usize = 256;

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
    let at = PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned());
    let masks = Masks::new();
    for (request, value) in values.into_iter().enumerate() {
        let checked = datasets
            .check(
                vec![PushedRecord {
                    dataset: "books".to_owned(),
                    data: value,
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

/// Writes the Export of `books` in `connection` in `format`, a page at a
/// time, and answers its size in bytes.
pub(crate) fn export(connection: &Connection, format: ExportFormat) -> usize {
    let options = ExportOptions::of("books");
    let mut encoder = ExportEncoder::new(format, &options, Columns::new(options.nested(format)));
    let mut bytes = Vec::new();
    encoder.begin(&mut bytes);
    let mut after = 0;
    loop {
        let page = read_values(
            connection,
            Query {
                dataset: "books",
                after,
                through: None,
                wall: None,
            },
            PAGE,
        )
        .expect("invariant: the Records read back");
        let full = page.len() == PAGE;
        for record in &page {
            encoder.value(
                &Exported {
                    number: record.number,
                    value: &record.value,
                    wall: record.wall.as_deref(),
                    committed_at: &record.committed_at,
                },
                &mut bytes,
            );
            after = record.number;
        }
        if !full {
            break;
        }
    }
    encoder.end(&mut bytes);
    bytes.len()
}
