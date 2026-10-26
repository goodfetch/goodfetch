//! The Export profile's writer for the Compatibility corpus (ADR 0066):
//! declares `books`, commits three Records through the Datasets' own
//! commit at ADR 0059's test `Clock` start, the shared Unicode record the
//! second, in a run file in memory, then writes their Export in each format
//! through the encoder `run.export` uses, `books.json` and `books.jsonl`,
//! into the folder given as the only argument. `xtask compat write` runs
//! it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_dataset::{Committing, Datasets, ExportEncoder, Query, TABLES, commit, read_values};
use goodfetch_engine::mask::Masks;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::export::ExportFormat;
use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use goodfetch_protocol::run::PointInTime;
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
        json!({ "title": "Café", "price": 12.5, "in_stock": true, "tags": ["roman", "poche"] }),
        unicode,
        json!({ "title": "Ünïcode", "price": null, "in_stock": false, "tags": [] }),
    ];
    let at = PointInTime::from_rfc3339(START.to_owned());
    for (request, value) in values.into_iter().enumerate() {
        let checked = datasets
            .check(vec![PushedRecord {
                dataset: "books".to_owned(),
                data: value,
            }])
            .expect("invariant: each Record holds");
        let id = (request + 1).to_string();
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
                    wall: None,
                    payment: None,
                    robots_txt: None,
                    value_transforms: Vec::new(),
                },
                masks: &Masks::new(),
            },
            &checked,
        )
        .expect("invariant: each Record commits");
    }
    let committed = read_values(
        &connection,
        Query {
            dataset: "books",
            after: 0,
            through: None,
        },
        usize::MAX,
    )
    .expect("invariant: the Records read back");
    for (format, name) in [
        (ExportFormat::Json, "books.json"),
        (ExportFormat::Jsonl, "books.jsonl"),
    ] {
        let mut encoder = ExportEncoder::new(format);
        let mut bytes = Vec::new();
        encoder.begin(&mut bytes);
        for (_, value) in &committed {
            encoder.value(value, &mut bytes);
        }
        encoder.end(&mut bytes);
        std::fs::write(output.join(name), bytes).expect("invariant: the output folder is writable");
    }
}
