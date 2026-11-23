//! The Sink conformance cases (ADR 0026) on the SQLite Sink, Fenced: each
//! Record once, in number order per Dataset, with its delivery ID, Dataset,
//! number and run epoch, whatever kills, lost answers or zombie copies
//! come between.

use std::time::Duration;

use goodfetch_sink::{Class, DeliveryId, Driver, FailureKind, SqliteSink, Status};
use goodfetch_test_support::unicode_record;
use rstest::rstest;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::faults::{Fault, Faulty};
use crate::run_files::{TARGET, copy, items, numbers, record_texts, rows, run_file, sink};

/// The Sink's name in every test.
const SINK: &str = "warehouse";

/// A driver for the copy `run_epoch` of `run-1`, delivering `datasets`
/// three Records a batch.
fn driver<S: goodfetch_sink::Primitives>(sink: S, run_epoch: u64, datasets: &[&str]) -> Driver<S> {
    Driver::new(
        SINK,
        sink,
        copy(run_epoch),
        datasets.iter().map(|&dataset| dataset.to_owned()).collect(),
    )
    .with_batch_records(3)
}

fn destination() -> (TempDir, std::path::PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("destination.sqlite");
    (folder, file)
}

/// Every Record of both Datasets reaches the destination once, in number
/// order within its Dataset, and each row carries its delivery ID, its
/// Dataset and number, the Run and its epoch, and its commit time.
///
/// Covers: story:6-285
#[test]
fn each_record_lands_once_with_its_delivery_id_dataset_number_and_epoch() {
    let (_folder, file) = destination();
    let records = run_file(&[("products", items("p", 7)), ("reviews", items("r", 4))]);
    let mut delivering = driver(sink(&file), 1, &["products", "reviews"]);

    assert_eq!(delivering.deliver(&records).unwrap(), Status::Complete);

    assert_eq!(delivering.class(), Some(Class::Fenced));
    let rows = rows(&file);
    let expected: Vec<(String, u64)> = (1..=7)
        .map(|n| ("products".to_owned(), n))
        .chain((1..=4).map(|n| ("reviews".to_owned(), n)))
        .collect();
    assert_eq!(numbers(&file), expected);
    for row in &rows {
        assert_eq!(
            row.delivery_id,
            DeliveryId::new("run-1", SINK, &row.dataset, row.number).to_string()
        );
        assert_eq!((row.run_id.as_str(), row.run_epoch), ("run-1", 1));
        assert_eq!(row.committed_at, "2026-01-05T09:00:00Z");
    }
    assert_eq!(rows[6].record, json!({ "item": "p7" }));
    assert_eq!(rows[7].record, json!({ "item": "r1" }));
}

/// Records committed after a delivery are delivered by the next one, after
/// those already delivered.
///
/// Covers: story:6-285
#[test]
fn a_later_delivery_takes_up_after_the_last_record_delivered() {
    let (_folder, file) = destination();
    let records = run_file(&[("products", items("p", 2))]);
    let mut delivering = driver(sink(&file), 1, &["products"]);
    assert_eq!(delivering.deliver(&records).unwrap(), Status::Complete);

    records
        .execute_batch(
            "INSERT INTO records VALUES ('products', 3, 'request-2', 'https://shop.test/3', '2026-01-05T09:01:00Z', '{\"item\":\"p3\"}', NULL, '{\"request_id\":\"request-2\"}')",
        )
        .unwrap();
    assert_eq!(delivering.deliver(&records).unwrap(), Status::Complete);

    assert_eq!(
        numbers(&file),
        vec![
            ("products".to_owned(), 1),
            ("products".to_owned(), 2),
            ("products".to_owned(), 3)
        ]
    );
}

/// An engine killed in the middle of a delivery, before or after its write
/// landed, and started again with the same run epoch, ends with every
/// Record at the destination once.
///
/// Covers: story:6-285
#[rstest]
#[case::before_the_write_landed(false)]
#[case::after_the_write_landed(true)]
fn a_kill_mid_delivery_loses_and_duplicates_nothing(#[case] landed: bool) {
    let (_folder, file) = destination();
    let records = run_file(&[("products", items("p", 8)), ("reviews", items("r", 5))]);
    let mut killed = driver(
        Faulty::new(sink(&file), Fault::Killed { write: 3, landed }),
        1,
        &["products", "reviews"],
    );
    assert!(matches!(
        killed.deliver(&records).unwrap(),
        Status::Behind { .. }
    ));
    drop(killed);

    let mut restarted = driver(sink(&file), 1, &["products", "reviews"]);
    assert_eq!(restarted.deliver(&records).unwrap(), Status::Complete);

    let expected: Vec<(String, u64)> = (1..=8)
        .map(|n| ("products".to_owned(), n))
        .chain((1..=5).map(|n| ("reviews".to_owned(), n)))
        .collect();
    assert_eq!(numbers(&file), expected);
}

/// A write that landed but whose answer was lost is sent again, and the
/// destination writes nothing twice: the batch's rows stay as first
/// written.
///
/// Covers: story:6-285
#[test]
fn a_retry_whose_write_had_landed_writes_nothing_twice() {
    let (_folder, file) = destination();
    let records = run_file(&[("products", items("p", 5))]);
    let mut delivering = driver(
        Faulty::new(sink(&file), Fault::AnswerLost(1)),
        1,
        &["products"],
    );

    let behind = delivering.deliver(&records).unwrap();
    let Status::Behind { failure, retry_in } = behind else {
        panic!("a lost answer leaves the Sink behind, not {behind:?}");
    };
    assert_eq!(failure.kind(), FailureKind::Transient);
    assert_eq!(retry_in, Duration::from_secs(5));
    let landed = rows(&file);
    assert_eq!(landed.len(), 3);

    assert_eq!(delivering.deliver(&records).unwrap(), Status::Complete);

    let rows = rows(&file);
    assert_eq!(rows[..3], landed[..]);
    assert_eq!(
        numbers(&file),
        (1..=5)
            .map(|n| ("products".to_owned(), n))
            .collect::<Vec<_>>()
    );
}

/// Two copies of one Run with different run epochs, the older a zombie
/// that keeps delivering: the destination ends with the current copy's
/// Records, and the zombie's later writes are refused.
///
/// Covers: story:6-285
#[test]
fn a_zombie_copy_never_overwrites_the_records_of_the_current_copy() {
    let (_folder, file) = destination();
    let zombie_records = run_file(&[("products", items("zombie-", 4))]);
    let current_records = run_file(&[("products", items("current-", 6))]);
    let mut zombie = driver(sink(&file), 1, &["products"]);
    let mut current = driver(sink(&file), 2, &["products"]);

    assert_eq!(zombie.deliver(&zombie_records).unwrap(), Status::Complete);
    assert_eq!(current.deliver(&current_records).unwrap(), Status::Complete);
    zombie_records
        .execute_batch(
            "INSERT INTO records VALUES ('products', 5, 'request-5', 'https://shop.test/5', '2026-01-05T09:01:00Z', '{\"item\":\"zombie-5\"}', NULL, '{\"request_id\":\"request-5\"}'),
                                       ('products', 6, 'request-6', 'https://shop.test/6', '2026-01-05T09:01:00Z', '{\"item\":\"zombie-6\"}', NULL, '{\"request_id\":\"request-6\"}'),
                                       ('products', 7, 'request-7', 'https://shop.test/7', '2026-01-05T09:01:00Z', '{\"item\":\"zombie-7\"}', NULL, '{\"request_id\":\"request-7\"}')",
        )
        .unwrap();
    assert_eq!(
        zombie.deliver(&zombie_records).unwrap(),
        Status::Superseded { run_epoch: 2 }
    );

    let rows = rows(&file);
    assert_eq!(
        rows.iter()
            .map(|row| (row.number, row.run_epoch, row.record.clone()))
            .collect::<Vec<_>>(),
        (1..=6)
            .map(|n| (n, 2, json!({ "item": format!("current-{n}") })))
            .collect::<Vec<_>>()
    );
}

/// The Unicode record's fields reach the destination byte for byte.
///
/// Covers: story:6-285
#[test]
fn the_unicode_record_round_trips_into_sqlite_byte_for_byte() {
    let (_folder, file) = destination();
    let unicode = unicode_record();
    let value: Value = unicode
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let records = run_file(&[("texts", vec![value])]);
    let mut delivering = driver(sink(&file), 1, &["texts"]);

    assert_eq!(delivering.deliver(&records).unwrap(), Status::Complete);

    let texts = record_texts(&file);
    let held: serde_json::Map<String, Value> = serde_json::from_str(&texts[0]).unwrap();
    for field in &unicode.fields {
        assert_eq!(
            held[&field.name].as_str().map(str::as_bytes),
            Some(field.text.as_bytes()),
            "the field {}",
            field.name
        );
    }
}

/// A Sink whose destination the engine may not write is failed, not
/// behind: no retry would deliver.
#[test]
fn a_destination_the_engine_may_not_write_fails_the_sink() {
    let (_folder, file) = destination();
    drop(rusqlite::Connection::open(&file).unwrap());
    let read_only =
        rusqlite::Connection::open_with_flags(&file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let records = run_file(&[("products", items("p", 1))]);
    let mut delivering = driver(SqliteSink::new(read_only, TARGET), 1, &["products"]);

    let status = delivering.deliver(&records).unwrap();

    let Status::Failed(failure) = status else {
        panic!("a destination the engine may not write fails the Sink, not {status:?}");
    };
    assert_eq!(failure.kind(), FailureKind::Auth);
}
