//! Conformance tests for delivery outliving the Run (ADR 0026): redelivering
//! one delivery ID while preserving deduplication, rejecting mismatched Run
//! IDs, and backfilling a past Run into a Sink configured after the Run ended.

use goodfetch_engine::mask::Masks;
use goodfetch_sink::{Driver, RunCopy, SqliteSink, Status, dead_letters};
use rusqlite::Connection;
use serde_json::json;

use crate::run_files::{commit, run_file};

fn copy_of(run_id: &str, run_epoch: u64) -> RunCopy {
    RunCopy {
        run_id: run_id.to_owned(),
        run_epoch,
    }
}

fn target_rows(dest: &Connection) -> Vec<(String, i64, String)> {
    dest.prepare(
        "SELECT _goodfetch_delivery_id, _goodfetch_record_number, url
         FROM records ORDER BY _goodfetch_record_number",
    )
    .unwrap()
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap()
}

/// Redelivering a single delivery ID (from dead letters or from a committed
/// Dataset row) preserves the exact `delivery_id` so the destination
/// deduplicates it, and ignores IDs belonging to another Run (ADR 0026).
///
/// Covers: story:6-292
#[test]
fn redeliver_preserves_delivery_id_and_deduplicates() {
    let run = run_file(&[]);
    dead_letters::ensure_table(&run).unwrap();
    commit(
        &run,
        "pages",
        1,
        &[
            json!({ "url": "https://a.test/1" }),
            json!({ "url": "https://a.test/2" }),
        ],
    );

    let dest = Connection::open_in_memory().unwrap();
    dest.execute_batch(
        "CREATE TABLE records (
             _goodfetch_delivery_id TEXT NOT NULL PRIMARY KEY,
             _goodfetch_run_id TEXT NOT NULL,
             _goodfetch_run_epoch INTEGER NOT NULL,
             _goodfetch_dataset TEXT NOT NULL,
             _goodfetch_record_number INTEGER NOT NULL,
             _goodfetch_committed_at TEXT NOT NULL,
             _goodfetch_meta TEXT,
             _goodfetch_key TEXT,
             url TEXT CHECK (url NOT LIKE '%/2')
         ) STRICT;",
    )
    .unwrap();

    let mut driver = Driver::new(
        "warehouse",
        SqliteSink::new(dest, "records"),
        copy_of("run-outlive", 1),
        vec!["pages".to_owned()],
    );
    assert_eq!(
        driver.deliver(&run).unwrap(),
        Status::CompleteWithRejections
    );
    let dl = driver.take_dead_letters();
    assert_eq!(dl.len(), 1);
    for entry in &dl {
        dead_letters::record(&run, entry, &Masks::new()).unwrap();
    }

    // Lift the CHECK constraint by opening a fresh table with identical schema
    // minus the constraint.
    let dir = tempfile::tempdir().unwrap();
    let dest2_path = dir.path().join("dest2.sqlite");
    let dest2 = Connection::open(&dest2_path).unwrap();
    let mut driver2 = Driver::new(
        "warehouse",
        SqliteSink::new(dest2, "records"),
        copy_of("run-outlive", 1),
        vec!["pages".to_owned()],
    );

    // Mismatched run_id is not found and writes nothing.
    let wrong = driver2
        .redeliver(&run, "other-run/warehouse/pages/1")
        .unwrap();
    assert!(!wrong.found);
    assert!(!wrong.delivered);

    // Redeliver the dead letter (record 2) preserving its exact delivery_id.
    let dl_id = dl[0].delivery_id.as_str().to_owned();
    let from_dl = driver2.redeliver(&run, &dl_id).unwrap();
    assert!(from_dl.found);
    assert!(from_dl.delivered);
    assert!(from_dl.still_rejected.is_none());

    // Redeliver committed record 1 by its delivery_id, twice: destination dedups.
    let rec1_id = "run-outlive/warehouse/pages/1";
    let first = driver2.redeliver(&run, rec1_id).unwrap();
    assert!(first.found && first.delivered);
    let again = driver2.redeliver(&run, rec1_id).unwrap();
    assert!(again.found && again.delivered);

    drop(driver2);
    let read_dest = Connection::open(&dest2_path).unwrap();
    let rows = target_rows(&read_dest);
    assert_eq!(
        rows,
        vec![
            (rec1_id.to_owned(), 1, "https://a.test/1".to_owned()),
            (dl_id, 2, "https://a.test/2".to_owned()),
        ]
    );
}

/// Backfilling a past Run into a Sink added after the Run ended delivers every
/// committed Record from Record 1 in order (ADR 0026).
///
/// Covers: story:6-292
#[test]
fn backfill_delivers_all_records_to_a_sink_added_later() {
    let run = run_file(&[]);
    commit(
        &run,
        "pages",
        1,
        &[
            json!({ "url": "https://a.test/1" }),
            json!({ "url": "https://a.test/2" }),
            json!({ "url": "https://a.test/3" }),
        ],
    );

    let dir = tempfile::tempdir().unwrap();
    let dest_path = dir.path().join("backfill.sqlite");
    let dest = Connection::open(&dest_path).unwrap();
    let mut driver = Driver::new(
        "added_later",
        SqliteSink::new(dest, "records"),
        copy_of("run-past", 1),
        vec!["pages".to_owned()],
    );
    assert_eq!(driver.backfill(&run).unwrap(), Status::Complete);
    assert_eq!(driver.delivered_count(), 3);
    drop(driver);
    let read_dest = Connection::open(&dest_path).unwrap();
    assert_eq!(
        target_rows(&read_dest),
        vec![
            (
                "run-past/added_later/pages/1".to_owned(),
                1,
                "https://a.test/1".to_owned()
            ),
            (
                "run-past/added_later/pages/2".to_owned(),
                2,
                "https://a.test/2".to_owned()
            ),
            (
                "run-past/added_later/pages/3".to_owned(),
                3,
                "https://a.test/3".to_owned()
            ),
        ]
    );
}
