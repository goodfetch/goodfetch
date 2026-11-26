//! A crawl's Sinks (ADRs 0026 and 0103), on Foxhound's executable over
//! stdio against the Proving ground: `sinks.<name>.*` on the crawl's Run
//! names where its Datasets' Records go, here the in-binary SQLite Sink
//! whose `destination` is a Secret reference to the database's path, and
//! the crawl itself stays the same.

use std::path::Path;

use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::code_free::crawl_read;

/// The Proving ground's seed.
const SEED: u64 = 994;

/// The Markdown task, whose index links its Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// The variable the Sink's `destination` reads the database's path from,
/// outside the `GOODFETCH_` prefix that ADR 0034 reserves.
const DESTINATION: &str = "SINK_TEST_DATABASE";

/// One row the SQLite Sink wrote.
#[derive(Debug, PartialEq)]
struct Row {
    delivery_id: String,
    run_id: String,
    run_epoch: i64,
    dataset: String,
    number: i64,
    record: Value,
}

/// The rows of the table `records` of the database at `path`, by Record
/// number.
fn rows(path: &Path) -> Vec<Row> {
    let connection = Connection::open(path).unwrap();
    let mut select = connection
        .prepare(
            "SELECT _goodfetch_delivery_id, _goodfetch_run_id, _goodfetch_run_epoch,
                    _goodfetch_dataset, _goodfetch_record_number, record
             FROM records ORDER BY _goodfetch_record_number",
        )
        .unwrap();
    select
        .query_map((), |row| {
            let record: String = row.get(5)?;
            Ok(Row {
                delivery_id: row.get(0)?,
                run_id: row.get(1)?,
                run_epoch: row.get(2)?,
                dataset: row.get(3)?,
                number: row.get(4)?,
                record: serde_json::from_str(&record).unwrap(),
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A code-free crawl whose Run names a SQLite Sink of its Dataset `pages`
/// ends with every Record of `pages` in the Sink's table, each row
/// carrying its delivery ID, its Dataset and number and the run epoch: set
/// on the Run, the Sink moves the data and the crawl's own declaration is
/// unchanged.
///
/// Covers: story:6-282, story:6-285, operation:crawl.start
#[test]
fn a_crawls_records_reach_the_sqlite_sink_its_run_names() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let database = env.folder().join("sink.sqlite");
    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database);
    });
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        (
            "delivered",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": format!("{index}/headings/levels"), "label": "page" }),
            ],
        ),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "http" },
            "sinks": {
                "mine": {
                    "type": "sqlite",
                    "datasets": ["pages"],
                    "destination": format!("env:{DESTINATION}"),
                },
            },
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    assert!(engine.finished());

    let epoch: i64 = crate::run_file(&env, "delivered")
        .prepare_cached("SELECT epoch FROM run")
        .unwrap()
        .query_row((), |row| row.get(0))
        .unwrap();
    let expected: Vec<Row> = records
        .iter()
        .map(|record| {
            let number = record["number"].as_i64().unwrap();
            Row {
                delivery_id: format!("delivered/mine/pages/{number}"),
                run_id: "delivered".to_owned(),
                run_epoch: epoch,
                dataset: "pages".to_owned(),
                number,
                record: record["value"].clone(),
            }
        })
        .collect();
    assert_eq!(expected.len(), 2, "{records:?}");
    assert_eq!(rows(&database), expected);
}

/// A Sink of a `type` this engine does not deliver to yet refuses the
/// crawl at its start with `capability_unavailable`, naming the entry's
/// `type`, rather than run with Records going nowhere.
///
/// Covers: story:6-282, operation:crawl.start
#[test]
fn a_sink_of_a_type_not_built_refuses_the_crawl_at_its_start() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::attached(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, _) = crawl_read(
        &mut engine,
        &proving_ground,
        ("refused", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "http" },
            "sinks": {
                "warehouse": {
                    "type": "postgres",
                    "datasets": ["pages"],
                    "destination": "env:WAREHOUSE_DSN",
                },
            },
        }),
    );

    let refused = &started["error"]["data"];
    assert_eq!(
        (&refused["code"], &refused["key"], &refused["kind"]),
        (
            &json!("capability_unavailable"),
            &json!("sinks.warehouse.type"),
            &json!("postgres"),
        ),
        "{started}"
    );
}
