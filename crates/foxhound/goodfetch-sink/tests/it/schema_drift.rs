//! Schema drift and contracts (ADR 0026): declared schema first, else
//! inferred; columns are only added, widened (`int` -> `float`), or split
//! into `<field>__v_<type>`; `evolve`, `freeze`, `discard_row` and
//! `discard_value` govern drift; names are normalized to `snake_case` with
//! collision suffixes; nested data goes as JSON or flattened when
//! `schema.flatten` is on; and every `_goodfetch_*` system column exists.

use std::collections::BTreeSet;
use std::path::Path;

use goodfetch_dataset::Datasets;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::sink::{SchemaContract, SinkSchema};
use goodfetch_sink::schema::{ColumnType, SchemaChange};
use goodfetch_sink::{Driver, RunCopy, SqliteSink, Status};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::run_files::{TARGET, commit_at, run_file};

const SINK: &str = "warehouse";
const FIRST: &str = "2026-01-05T09:00:00Z";

fn destination() -> (TempDir, std::path::PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("destination.sqlite");
    (folder, file)
}

fn copy_of(run_id: &str, run_epoch: u64) -> RunCopy {
    RunCopy {
        run_id: run_id.to_owned(),
        run_epoch,
    }
}

fn table_columns(path: &Path) -> Vec<(String, String)> {
    let conn = Connection::open(path).unwrap();
    conn.prepare(&format!(
        "SELECT name, type FROM pragma_table_info('{TARGET}') ORDER BY cid"
    ))
    .unwrap()
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap()
}

struct EvolvedRow {
    number: i64,
    name: String,
    price: Option<f64>,
    price_text: Option<String>,
    tags: Option<String>,
    details: Option<String>,
    in_stock: Option<i64>,
    in_stock_text: Option<String>,
}

fn read_evolved_rows(file: &std::path::Path) -> Vec<EvolvedRow> {
    let conn = Connection::open(file).unwrap();
    let mut query = conn
        .prepare(&format!(
            "SELECT _goodfetch_record_number, product_name, price, price__v_text, tags, details, in_stock, in_stock__v_text
             FROM \"{TARGET}\" ORDER BY _goodfetch_record_number"
        ))
        .unwrap();
    query
        .query_map([], |r| {
            Ok(EvolvedRow {
                number: r.get(0)?,
                name: r.get(1)?,
                price: r.get(2)?,
                price_text: r.get(3)?,
                tags: r.get(4)?,
                details: r.get(5)?,
                in_stock: r.get(6)?,
                in_stock_text: r.get(7)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn assert_evolve_changes_and_columns(changes: &[SchemaChange], file: &std::path::Path) {
    assert!(changes.iter().any(|change| matches!(
        change,
        SchemaChange::AddedColumn { name, .. } if name == "in_stock"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        SchemaChange::WidenedColumn {
            name,
            from: ColumnType::Integer,
            to: ColumnType::Float
        } if name == "price"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        SchemaChange::AddedVariantColumn { base, name, column_type: ColumnType::Text }
            if base == "price" && name == "price__v_text"
    )));

    let cols = table_columns(file);
    let col_names: BTreeSet<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
    for expected in [
        "_goodfetch_delivery_id",
        "_goodfetch_run_id",
        "_goodfetch_run_epoch",
        "_goodfetch_dataset",
        "_goodfetch_record_number",
        "_goodfetch_committed_at",
        "_goodfetch_meta",
        "_goodfetch_key",
        "product_name",
        "price",
        "tags",
        "details",
        "in_stock",
        "price__v_text",
        "in_stock__v_text",
    ] {
        assert!(col_names.contains(expected), "missing `{expected}`");
    }
    assert_eq!(
        cols.iter()
            .find(|(name, _)| name == "price")
            .map(|(_, ty)| ty.as_str()),
        Some("REAL")
    );
}

/// Mid-run schema drift under `evolve` adds columns, widens `int` ->
/// `float`, splits incompatible types into `<field>__v_<type>`, normalizes
/// names to `snake_case`, keeps nested data as JSON, includes every
/// `_goodfetch_*` system column, and never loses a Record silently.
///
/// Covers: story:6-288
#[test]
fn schema_drift_under_evolve_adds_widens_and_splits_without_losing_any_record() {
    let (_folder, file) = destination();
    let run = run_file(&[]);
    commit_at(
        &run,
        "products",
        1,
        &[json!({
            "productName": "Widget",
            "price": 10,
            "tags": ["sale", "new"],
            "details": { "color": "red" }
        })],
        FIRST,
    );

    let sqlite =
        SqliteSink::new(Connection::open(&file).unwrap(), TARGET).with_schema(SinkSchema {
            contract: Some(SchemaContract::Evolve),
            flatten: Some(false),
        });
    let mut driver = Driver::new(
        SINK,
        sqlite,
        copy_of("run-1", 1),
        vec!["products".to_owned()],
    );
    assert_eq!(driver.deliver(&run).unwrap(), Status::Complete);

    commit_at(
        &run,
        "products",
        2,
        &[json!({
            "productName": "Gadget",
            "price": 19.5,
            "inStock": true,
            "tags": ["hot"],
            "details": { "color": "blue" }
        })],
        FIRST,
    );
    assert_eq!(driver.deliver(&run).unwrap(), Status::Complete);

    commit_at(
        &run,
        "products",
        3,
        &[json!({
            "productName": "Freebie",
            "price": "free",
            "inStock": "yes"
        })],
        FIRST,
    );
    assert_eq!(driver.deliver(&run).unwrap(), Status::Complete);

    let sqlite = driver.into_sink();
    assert_evolve_changes_and_columns(sqlite.schema_changes(), &file);

    let rows = read_evolved_rows(&file);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (rows[0].number, rows[0].name.as_str(), rows[0].price),
        (1, "Widget", Some(10.0))
    );
    assert_eq!(rows[0].price_text, None);
    assert_eq!(
        serde_json::from_str::<Value>(rows[0].tags.as_deref().unwrap()).unwrap(),
        json!(["sale", "new"])
    );
    assert_eq!(
        serde_json::from_str::<Value>(rows[0].details.as_deref().unwrap()).unwrap(),
        json!({"color": "red"})
    );
    assert_eq!(
        (
            rows[1].number,
            rows[1].name.as_str(),
            rows[1].price,
            rows[1].in_stock
        ),
        (2, "Gadget", Some(19.5), Some(1))
    );
    assert_eq!(
        (
            rows[2].number,
            rows[2].name.as_str(),
            rows[2].price,
            rows[2].price_text.as_deref(),
            rows[2].in_stock,
            rows[2].in_stock_text.as_deref(),
        ),
        (3, "Freebie", None, Some("free"), None, Some("yes"))
    );
}

/// With a declared Dataset schema, `freeze` refuses drifting Records into
/// dead letters, `discard_row` drops the row with a count, and
/// `discard_value` drops only the drifting value with a count and a
/// `_goodfetch_meta` entry.
///
/// Covers: story:6-288
#[rstest]
#[case::freeze(SchemaContract::Freeze)]
#[case::discard_row(SchemaContract::DiscardRow)]
#[case::discard_value(SchemaContract::DiscardValue)]
fn declared_schema_is_used_first_and_contracts_govern_drift(#[case] contract: SchemaContract) {
    let (_folder, file) = destination();
    let run = run_file(&[]);
    let declarations = [DatasetDeclaration {
        name: "products".to_owned(),
        schema: Some(json!({
            "type": "object",
            "properties": {
                "sku": { "type": "string" },
                "price": { "type": "integer" }
            },
            "required": ["sku", "price"]
        })),
        key: vec!["sku".to_owned()],
    }];
    Datasets::declare(&declarations)
        .unwrap()
        .record(&run)
        .unwrap();

    commit_at(
        &run,
        "products",
        1,
        &[
            json!({ "sku": "a", "price": 10 }),
            json!({ "sku": "b", "price": 20, "extraField": "unannounced" }),
            json!({ "sku": "c", "price": 30 }),
        ],
        FIRST,
    );

    let sqlite =
        SqliteSink::new(Connection::open(&file).unwrap(), TARGET).with_schema(SinkSchema {
            contract: Some(contract),
            flatten: Some(false),
        });
    let mut driver = Driver::new(
        SINK,
        sqlite,
        copy_of("run-1", 1),
        vec!["products".to_owned()],
    );
    let status = driver.deliver(&run).unwrap();
    let dead = driver.take_dead_letters();
    let sqlite = driver.into_sink();
    let counters = sqlite.schema_counters();

    let cols = table_columns(&file);
    assert!(
        !cols.iter().any(|(name, _)| name == "extra_field"),
        "non-evolve contract {contract:?} must not add extra_field: {cols:?}"
    );

    let conn = Connection::open(&file).unwrap();
    let landed: Vec<(String, i64, Option<String>)> = conn
        .prepare(&format!(
            "SELECT sku, price, _goodfetch_meta FROM \"{TARGET}\" ORDER BY _goodfetch_record_number"
        ))
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    match contract {
        SchemaContract::Freeze => {
            assert_eq!(status, Status::CompleteWithRejections);
            assert_eq!(dead.len(), 1);
            assert_eq!(dead[0].number, 2);
            assert_eq!(landed.len(), 2);
            assert_eq!(landed[0].0, "a");
            assert_eq!(landed[1].0, "c");
        }
        SchemaContract::DiscardRow => {
            assert_eq!(status, Status::Complete);
            assert_eq!(dead, Vec::new());
            assert_eq!(counters.discarded_rows, 1);
            assert_eq!(counters.discarded_values, 0);
            assert_eq!(landed.len(), 2);
            assert_eq!(landed[0].0, "a");
            assert_eq!(landed[1].0, "c");
        }
        SchemaContract::DiscardValue => {
            assert_eq!(status, Status::Complete);
            assert_eq!(dead, Vec::new());
            assert_eq!(counters.discarded_rows, 0);
            assert_eq!(counters.discarded_values, 1);
            assert_eq!(landed.len(), 3);
            assert_eq!(landed[1].0, "b");
            assert_eq!(landed[1].1, 20);
            let meta: Value = serde_json::from_str(landed[1].2.as_deref().unwrap()).unwrap();
            assert_eq!(meta["discarded_values"], json!(["extraField"]));
        }
        SchemaContract::Evolve => unreachable!(),
    }
}

/// Column names normalize to `snake_case` with deterministic collision
/// suffixes, and `schema.flatten: true` flattens nested objects while
/// keeping arrays as JSON.
///
/// Covers: story:6-288
#[test]
fn names_normalize_with_collision_suffixes_and_flatten_expands_nested_objects() {
    let (_folder, file) = destination();
    let run = run_file(&[]);
    commit_at(
        &run,
        "books",
        1,
        &[json!({
            "firstName": "Frank",
            "first_name": "Herbert",
            "HTTPStatus": 200,
            "123code": "ok",
            "author": { "fullName": "Frank Herbert", "born": 1920 },
            "tags": ["sci-fi", "classic"]
        })],
        FIRST,
    );

    let sqlite =
        SqliteSink::new(Connection::open(&file).unwrap(), TARGET).with_schema(SinkSchema {
            contract: Some(SchemaContract::Evolve),
            flatten: Some(true),
        });
    let mut driver = Driver::new(SINK, sqlite, copy_of("run-1", 1), vec!["books".to_owned()]);
    assert_eq!(driver.deliver(&run).unwrap(), Status::Complete);

    let cols = table_columns(&file);
    let col_names: BTreeSet<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
    for expected in [
        "first_name",
        "first_name_2",
        "http_status",
        "f_123code",
        "author_full_name",
        "author_born",
        "tags",
    ] {
        assert!(
            col_names.contains(expected),
            "expected column `{expected}` in {col_names:?}"
        );
    }

    let conn = Connection::open(&file).unwrap();
    let (full_name, born, tags): (String, i64, String) = conn
        .prepare_cached(&format!(
            "SELECT author_full_name, author_born, tags FROM \"{TARGET}\""
        ))
        .unwrap()
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert_eq!(full_name, "Frank Herbert");
    assert_eq!(born, 1920);
    assert_eq!(
        serde_json::from_str::<Value>(&tags).unwrap(),
        json!(["sci-fi", "classic"])
    );
}
