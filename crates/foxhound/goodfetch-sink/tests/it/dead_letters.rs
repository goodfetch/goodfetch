//! Batch splitting and dead letters (ADR 0026): a rejected batch is split
//! until every bad Record is isolated, every good Record is delivered in
//! order, the bad Records join the Sink's dead-letter list with the
//! destination's masked reason, and a later Run retries them and removes
//! each on success.

use std::collections::BTreeMap;

use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::sink::WriteMode;
use goodfetch_sink::dead_letters::{self, DeadLetter};
use goodfetch_sink::{
    Batch, DeliveryId, Driver, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy,
    Status, Written,
};
use goodfetch_test_support::{Canaries, Output, SecretKind};
use rusqlite::Connection;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::run_files::{TARGET, commit_at, run_file, sink};

/// The Sink's name in every test.
const SINK: &str = "warehouse";

/// When the first Run committed its Records.
const FIRST: &str = "2026-01-05T09:00:00Z";

/// A model destination that rejects any batch containing a Record whose
/// `"bad"` field equals `reject_tag`, with a chosen destination error text,
/// recording every batch size it saw and every Record it accepted.
struct RejectingDestination {
    reject_tag: Option<String>,
    reason: String,
    batch_sizes: Vec<usize>,
    accepted: Vec<(DeliveryId, Value)>,
    delivered: BTreeMap<String, u64>,
}

impl RejectingDestination {
    fn rejecting(tag: &str, reason: &str) -> Self {
        Self {
            reject_tag: Some(tag.to_owned()),
            reason: reason.to_owned(),
            batch_sizes: Vec::new(),
            accepted: Vec::new(),
            delivered: BTreeMap::new(),
        }
    }

    fn accepting_all(&mut self) {
        self.reject_tag = None;
    }
}

impl Primitives for RejectingDestination {
    type Error = (FailureKind, String);

    fn probe(&mut self, _run: &RunCopy) -> Result<Probe, Self::Error> {
        Ok(Probe {
            class: GuaranteeClass::Fenced,
            delivered: self.delivered.clone(),
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        self.batch_sizes.push(batch.records.len());
        if let Some(tag) = &self.reject_tag
            && batch
                .records
                .iter()
                .any(|record| record.value.get("bad").and_then(Value::as_str) == Some(tag.as_str()))
        {
            return Err((FailureKind::RejectedRecords, self.reason.clone()));
        }
        for record in &batch.records {
            self.accepted
                .push((record.delivery_id.clone(), record.value.clone()));
            let entry = self.delivered.entry(batch.dataset.clone()).or_insert(0);
            *entry = (*entry).max(record.number);
        }
        Ok(Written::Delivered)
    }

    fn map_failure(&self, (kind, text): Self::Error) -> Failure {
        Failure::new(kind, text)
    }
}

fn copy_of(run_id: &str, run_epoch: u64) -> RunCopy {
    RunCopy {
        run_id: run_id.to_owned(),
        run_epoch,
    }
}

/// A batch holding one bad Record is split recursively until the bad
/// Record is isolated; every good Record is delivered in order, and the
/// bad Record lands on the Sink's dead-letter list with the destination's
/// reason.
///
/// Covers: story:6-289
#[test]
fn a_rejected_batch_splits_until_the_bad_record_is_isolated_and_dead_lettered() {
    let run_file = run_file(&[]);
    dead_letters::ensure_table(&run_file).unwrap();
    commit_at(
        &run_file,
        "products",
        1,
        &[
            json!({"sku": "a"}),
            json!({"sku": "b"}),
            json!({"sku": "c", "bad": "poison"}),
            json!({"sku": "d"}),
            json!({"sku": "e"}),
        ],
        FIRST,
    );

    let destination = RejectingDestination::rejecting("poison", "check constraint: negative price");
    let mut driver = Driver::new(
        SINK,
        destination,
        copy_of("run-1", 1),
        vec!["products".to_owned()],
    );
    let status = driver.deliver(&run_file).unwrap();

    assert_eq!(status, Status::CompleteWithRejections);
    let rejected = driver.take_dead_letters();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].number, 3);
    assert_eq!(rejected[0].dataset, "products");
    assert_eq!(rejected[0].reason, "check constraint: negative price");
    assert_eq!(rejected[0].attempts, 1);

    let masks = Masks::new();
    for entry in &rejected {
        dead_letters::record(&run_file, entry, &masks).unwrap();
    }
    let stored = dead_letters::list(&run_file, SINK).unwrap();
    assert_eq!(stored, rejected);
    assert_eq!(dead_letters::count(&run_file, SINK).unwrap(), 1);

    // Every good Record landed in number order, and the batch was split.
    let destination = driver.into_sink();
    assert!(destination.batch_sizes.contains(&5));
    assert!(destination.batch_sizes.contains(&1));
    let skus: Vec<&str> = destination
        .accepted
        .iter()
        .map(|(_, value)| value["sku"].as_str().unwrap())
        .collect();
    assert_eq!(skus, vec!["a", "b", "d", "e"]);
}

/// On the real SQLite Sink, a destination constraint rejecting one Record
/// splits the batch so every good Record lands in the table and the bad
/// Record is isolated with SQLite's reason.
///
/// Covers: story:6-289
#[test]
fn a_sqlite_constraint_rejecting_one_record_delivers_good_records_and_dead_letters_the_bad_one() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("destination.sqlite");
    // Pre-create the target table with a CHECK constraint rejecting `"bad":true`.
    {
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE \"{TARGET}\" (
                 _goodfetch_delivery_id TEXT NOT NULL PRIMARY KEY,
                 _goodfetch_run_id TEXT NOT NULL,
                 _goodfetch_run_epoch INTEGER NOT NULL,
                 _goodfetch_dataset TEXT NOT NULL,
                 _goodfetch_record_number INTEGER NOT NULL,
                 _goodfetch_committed_at TEXT NOT NULL,
                 record TEXT NOT NULL CHECK (record NOT LIKE '%\"bad\":true%'),
                 _goodfetch_key TEXT
             ) STRICT;"
        ))
        .unwrap();
    }

    let run_file = run_file(&[]);
    dead_letters::ensure_table(&run_file).unwrap();
    commit_at(
        &run_file,
        "products",
        1,
        &[
            json!({"sku": "a"}),
            json!({"sku": "b", "bad": true}),
            json!({"sku": "c"}),
            json!({"sku": "d"}),
        ],
        FIRST,
    );

    let mut driver = Driver::new(
        SINK,
        sink(&file).with_mode(WriteMode::Append),
        copy_of("run-1", 1),
        vec!["products".to_owned()],
    );
    let status = driver.deliver(&run_file).unwrap();
    assert_eq!(status, Status::CompleteWithRejections);

    let rejected = driver.take_dead_letters();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].number, 2);
    assert_ne!(
        rejected[0].reason, "",
        "SQLite's rejection reason is kept on the dead letter"
    );

    let conn = Connection::open(&file).unwrap();
    let landed: Vec<String> = conn
        .prepare(&format!(
            "SELECT json_extract(record, '$.sku') FROM \"{TARGET}\" ORDER BY _goodfetch_record_number"
        ))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(landed, vec!["a", "c", "d"]);
}

/// Dead letters recorded by an earlier Run are retried by a later Run:
/// one that still fails stays with an incremented attempt count, and one
/// that succeeds lands in the destination and is removed from the list.
///
/// Covers: story:6-289
#[test]
fn dead_letters_are_retried_on_a_later_run_and_removed_on_success() {
    let run_file = run_file(&[]);
    dead_letters::ensure_table(&run_file).unwrap();
    commit_at(
        &run_file,
        "products",
        1,
        &[json!({"sku": "a"}), json!({"sku": "b", "bad": "poison"})],
        FIRST,
    );

    let masks = Masks::new();
    let destination = RejectingDestination::rejecting("poison", "rejected on run 1");
    let mut run1 = Driver::new(
        SINK,
        destination,
        copy_of("run-1", 1),
        vec!["products".to_owned()],
    );
    assert_eq!(
        run1.deliver(&run_file).unwrap(),
        Status::CompleteWithRejections
    );
    for entry in run1.take_dead_letters() {
        dead_letters::record(&run_file, &entry, &masks).unwrap();
    }
    let mut destination = run1.into_sink();
    assert_eq!(dead_letters::count(&run_file, SINK).unwrap(), 1);

    // Run 2 retries while the destination still rejects it -> stays with attempts = 2.
    destination.reason = "still rejected on run 2".to_owned();
    let mut run2 = Driver::new(
        SINK,
        destination,
        copy_of("run-2", 1),
        vec!["products".to_owned()],
    );
    let pending = dead_letters::list(&run_file, SINK).unwrap();
    let outcome = run2.retry_dead_letters(&run_file, &pending).unwrap();
    assert_eq!(outcome.delivered, Vec::<DeliveryId>::new());
    assert_eq!(outcome.still_rejected.len(), 1);
    for entry in &outcome.still_rejected {
        dead_letters::record(&run_file, entry, &masks).unwrap();
    }
    let stored = dead_letters::list(&run_file, SINK).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].attempts, 2);
    assert_eq!(stored[0].reason, "still rejected on run 2");

    // Destination is fixed; Run 3 retries -> succeeds and is removed.
    let mut destination = run2.into_sink();
    destination.accepting_all();
    let mut run3 = Driver::new(
        SINK,
        destination,
        copy_of("run-3", 1),
        vec!["products".to_owned()],
    );
    let pending = dead_letters::list(&run_file, SINK).unwrap();
    let outcome = run3.retry_dead_letters(&run_file, &pending).unwrap();
    assert_eq!(
        outcome.delivered,
        vec![DeliveryId::new("run-1", SINK, "products", 2)]
    );
    assert_eq!(outcome.still_rejected, Vec::<DeadLetter>::new());
    for id in &outcome.delivered {
        dead_letters::remove(&run_file, SINK, id).unwrap();
    }
    assert_eq!(
        dead_letters::list(&run_file, SINK).unwrap(),
        Vec::<DeadLetter>::new()
    );
    let destination = run3.into_sink();
    assert_eq!(destination.accepted.len(), 2);
    assert_eq!(
        destination.accepted[1].0,
        DeliveryId::new("run-1", SINK, "products", 2)
    );
}

/// Secret canaries in a Record or echoed by a destination's error text are
/// masked before they reach the dead-letter table (ADR 0067).
///
/// Covers: story:6-289
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "synchronous test reads its temporary SQLite file for the Secret canary scan"
)]
fn secret_canaries_are_masked_in_dead_letters() {
    let mut canaries = Canaries::new(1139);
    let record_secret = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let dsn_secret = canaries.plant(SecretKind::ProxyPassword).value().to_owned();

    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join(
        "record_secret",
        &SecretString::from(record_secret.clone()),
        Reach::Value,
    );
    run_mask.join(
        "dsn_secret",
        &SecretString::from(dsn_secret.clone()),
        Reach::Value,
    );

    let run = run_file(&[]);
    commit_at(
        &run,
        "products",
        1,
        &[json!({"sku": "a", "bad": "leak", "token": record_secret})],
        FIRST,
    );

    let folder = tempfile::tempdir().unwrap();
    let db_path = folder.path().join("dead_letters.sqlite");
    let dl_conn = Connection::open(&db_path).unwrap();
    dead_letters::ensure_table(&dl_conn).unwrap();

    let destination = RejectingDestination::rejecting(
        "leak",
        &format!("destination rejected payload with token {dsn_secret}"),
    );
    let mut driver = Driver::new(
        SINK,
        destination,
        copy_of("run-1", 1),
        vec!["products".to_owned()],
    )
    .with_masks(masks.clone());
    assert_eq!(
        driver.deliver(&run).unwrap(),
        Status::CompleteWithRejections
    );
    for entry in driver.take_dead_letters() {
        dead_letters::record(&dl_conn, &entry, &masks).unwrap();
    }

    let stored = dead_letters::list(&dl_conn, SINK).unwrap();
    assert_eq!(stored.len(), 1);
    assert!(stored[0].reason.contains("%dsn_secret%"));
    assert_eq!(stored[0].value["token"], "%record_secret%");

    let serialized = format!("{stored:?}");
    let raw_bytes = std::fs::read(&db_path).unwrap();
    let outputs = [
        Output::Bytes {
            name: "dead_letters",
            bytes: serialized.as_bytes(),
        },
        Output::Bytes {
            name: "dead_letters.sqlite",
            bytes: &raw_bytes,
        },
    ];
    assert_eq!(canaries.leaks(&outputs).unwrap(), Vec::new());
}
