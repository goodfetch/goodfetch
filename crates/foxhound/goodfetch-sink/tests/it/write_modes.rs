//! The write modes of ADR 0026 on the SQLite Sink, keyed on the Sink's own
//! key or each Dataset's (ADR 0103): what the destination holds once two
//! Runs delivered the same product.

use std::path::Path;

use goodfetch_dataset::Datasets;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::sink::WriteMode;
use goodfetch_sink::{Driver, Failure, FailureKind, RunCopy, Status};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::run_files::{TARGET, commit_at, run_file, sink};

/// The Sink's name in every test.
const SINK: &str = "warehouse";

/// When the first Run committed its Records.
const FIRST: &str = "2026-01-05T09:00:00Z";

/// When the second Run committed its Records.
const SECOND: &str = "2026-01-06T09:00:00Z";

fn destination() -> (TempDir, std::path::PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("destination.sqlite");
    (folder, file)
}

/// A run file declaring each of `datasets`, a name and its key, with no
/// schema.
fn declared(datasets: &[(&str, &[&str])]) -> Connection {
    let connection = run_file(&[]);
    let declarations: Vec<DatasetDeclaration> = datasets
        .iter()
        .map(|(name, key)| DatasetDeclaration {
            name: (*name).to_owned(),
            schema: None,
            key: key.iter().map(|&field| field.to_owned()).collect(),
        })
        .collect();
    Datasets::declare(&declarations)
        .unwrap()
        .record(&connection)
        .unwrap();
    connection
}

/// The copy `run_epoch` of the Run `run_id`.
fn copy_of(run_id: &str, run_epoch: u64) -> RunCopy {
    RunCopy {
        run_id: run_id.to_owned(),
        run_epoch,
    }
}

/// Delivers what `run_file` committed to `datasets` for the copy `run` of
/// a Run, under `mode` keyed on the Sink's `own` key, as the Sink's run
/// opens it; then finishes the Run when `finished`, as its end does.
fn deliver(
    destination: &Path,
    mode: WriteMode,
    own: Option<Vec<String>>,
    run: RunCopy,
    datasets: &[&str],
    run_file: &Connection,
    finished: bool,
) -> Status {
    let driver = Driver::new(
        SINK,
        sink(destination).with_mode(mode),
        run,
        datasets.iter().map(|&dataset| dataset.to_owned()).collect(),
    );
    let mut driver = if mode == WriteMode::Append {
        driver
    } else {
        driver.keyed(own)
    };
    let delivered = driver.deliver(run_file).unwrap();
    if finished && delivered == Status::Complete {
        return driver.finish();
    }
    delivered
}

/// The target's rows, in the order first written: each one's Run, Dataset
/// and Record.
fn held(destination: &Path) -> Vec<(String, String, Value)> {
    let connection = Connection::open(destination).unwrap();
    let mut select = connection
        .prepare(&format!(
            "SELECT _goodfetch_run_id, _goodfetch_dataset, record FROM \"{TARGET}\" ORDER BY rowid"
        ))
        .unwrap();
    select
        .query_map((), |row| {
            let record: String = row.get(2)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                serde_json::from_str(&record).unwrap(),
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The target's versions under `history`: each one's Record, key and
/// valid-from and valid-to times, in the order first written.
fn versions(destination: &Path) -> Vec<(Value, String, String, Option<String>)> {
    let connection = Connection::open(destination).unwrap();
    let mut select = connection
        .prepare(&format!(
            "SELECT record, _goodfetch_key, _goodfetch_valid_from, _goodfetch_valid_to
             FROM \"{TARGET}\" ORDER BY rowid"
        ))
        .unwrap();
    select
        .query_map((), |row| {
            let record: String = row.get(0)?;
            Ok((
                serde_json::from_str(&record).unwrap(),
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The product `a` at 1 and `b` at 2, committed by the first Run, and `a`
/// at 3 by the second, keyed on the Dataset's `sku`.
fn two_runs() -> (Connection, Connection) {
    let first = declared(&[("products", &["sku"])]);
    commit_at(
        &first,
        "products",
        1,
        &[
            json!({"sku": "a", "price": 1}),
            json!({"sku": "b", "price": 2}),
        ],
        FIRST,
    );
    let second = declared(&[("products", &["sku"])]);
    commit_at(
        &second,
        "products",
        1,
        &[json!({"sku": "a", "price": 3})],
        SECOND,
    );
    (first, second)
}

/// The same product delivered by two finished Runs: `append` adds a row,
/// `upsert` keeps one row of it, `insert_only` ignores the second,
/// `replace` leaves the second Run's Records alone and `history` adds a
/// version.
///
/// Covers: story:6-286
#[rstest]
#[case::append(WriteMode::Append, &[("run-1", 1, "a"), ("run-1", 2, "b"), ("run-2", 3, "a")])]
#[case::upsert(WriteMode::Upsert, &[("run-2", 3, "a"), ("run-1", 2, "b")])]
#[case::insert_only(WriteMode::InsertOnly, &[("run-1", 1, "a"), ("run-1", 2, "b")])]
#[case::replace(WriteMode::Replace, &[("run-2", 3, "a")])]
#[case::history(WriteMode::History, &[("run-1", 1, "a"), ("run-1", 2, "b"), ("run-2", 3, "a")])]
fn the_same_product_from_two_runs_lands_as_its_write_mode_says(
    #[case] mode: WriteMode,
    #[case] expected: &[(&str, u64, &str)],
) {
    let (_folder, file) = destination();
    let (first, second) = two_runs();

    let delivered = [
        deliver(
            &file,
            mode,
            None,
            copy_of("run-1", 1),
            &["products"],
            &first,
            true,
        ),
        deliver(
            &file,
            mode,
            None,
            copy_of("run-2", 1),
            &["products"],
            &second,
            true,
        ),
    ];

    assert_eq!(delivered, [Status::Complete, Status::Complete]);
    let expected: Vec<(String, String, Value)> = expected
        .iter()
        .map(|&(run, price, sku)| {
            (
                run.to_owned(),
                "products".to_owned(),
                json!({"sku": sku, "price": price}),
            )
        })
        .collect();
    assert_eq!(held(&file), expected);
}

/// Under `history`, the second Run's product closes the first's version
/// at its own commit time and is valid from then; the product it did not
/// collect stays current.
///
/// Covers: story:6-286
#[test]
fn history_closes_the_earlier_version_where_the_later_begins() {
    let (_folder, file) = destination();
    let (first, second) = two_runs();

    for (run, run_file) in [("run-1", &first), ("run-2", &second)] {
        let delivered = deliver(
            &file,
            WriteMode::History,
            None,
            copy_of(run, 1),
            &["products"],
            run_file,
            true,
        );
        assert_eq!(delivered, Status::Complete);
    }

    assert_eq!(
        versions(&file),
        [
            (
                json!({"sku": "a", "price": 1}),
                r#"["a"]"#.to_owned(),
                FIRST.to_owned(),
                Some(SECOND.to_owned())
            ),
            (
                json!({"sku": "b", "price": 2}),
                r#"["b"]"#.to_owned(),
                FIRST.to_owned(),
                None
            ),
            (
                json!({"sku": "a", "price": 3}),
                r#"["a"]"#.to_owned(),
                SECOND.to_owned(),
                None
            ),
        ]
    );
}

/// Under `replace`, the target keeps what it held while a Run delivers and
/// until it finishes; its finish swaps the Run's Records in, and a second
/// finish, its answer lost, changes nothing; a Run that does not finish
/// never replaces the table.
///
/// Covers: story:6-286
#[test]
fn replace_swaps_a_finished_runs_records_in_at_its_end_once() {
    let (_folder, file) = destination();
    let (first, second) = two_runs();
    let first_held = || {
        vec![
            (
                "run-1".to_owned(),
                "products".to_owned(),
                json!({"sku": "a", "price": 1}),
            ),
            (
                "run-1".to_owned(),
                "products".to_owned(),
                json!({"sku": "b", "price": 2}),
            ),
        ]
    };
    deliver(
        &file,
        WriteMode::Replace,
        None,
        copy_of("run-1", 1),
        &["products"],
        &first,
        true,
    );
    let stopped = deliver(
        &file,
        WriteMode::Replace,
        None,
        copy_of("run-3", 1),
        &["products"],
        &second,
        false,
    );

    let mut driver = Driver::new(
        SINK,
        sink(&file).with_mode(WriteMode::Replace),
        copy_of("run-2", 1),
        vec!["products".to_owned()],
    )
    .keyed(None);
    let delivered = driver.deliver(&second).unwrap();
    let before_finish = held(&file);
    let finished = [driver.finish(), driver.finish()];

    assert_eq!((stopped, delivered), (Status::Complete, Status::Complete));
    assert_eq!(before_finish, first_held());
    assert_eq!(finished, [Status::Complete, Status::Complete]);
    assert_eq!(
        held(&file),
        [(
            "run-2".to_owned(),
            "products".to_owned(),
            json!({"sku": "a", "price": 3})
        )]
    );
}

/// A Sink fed two Datasets keyed differently keeps each Record of each
/// under its own Dataset's key: two reviews of one product stay two rows;
/// the Sink's own key, when it names one, keys both.
///
/// Covers: story:6-286
#[rstest]
#[case::each_datasets_key(None, &[("products", 3, "x"), ("reviews", 5, "x"), ("reviews", 2, "y")])]
#[case::the_sinks_own_key(Some(vec!["sku".to_owned()]), &[("products", 3, "x"), ("reviews", 5, "x")])]
fn each_dataset_is_keyed_on_its_own_key_unless_the_sink_names_one(
    #[case] own: Option<Vec<String>>,
    #[case] expected: &[(&str, u64, &str)],
) {
    let (_folder, file) = destination();
    let keys: &[(&str, &[&str])] = &[("products", &["sku"]), ("reviews", &["author", "sku"])];
    let first = declared(keys);
    commit_at(
        &first,
        "products",
        1,
        &[json!({"sku": "a", "author": "x", "price": 1})],
        FIRST,
    );
    commit_at(
        &first,
        "reviews",
        1,
        &[
            json!({"sku": "a", "author": "x", "price": 1}),
            json!({"sku": "a", "author": "y", "price": 2}),
        ],
        FIRST,
    );
    let second = declared(keys);
    commit_at(
        &second,
        "products",
        1,
        &[json!({"sku": "a", "author": "x", "price": 3})],
        SECOND,
    );
    commit_at(
        &second,
        "reviews",
        1,
        &[json!({"sku": "a", "author": "x", "price": 5})],
        SECOND,
    );

    for (run, run_file) in [("run-1", &first), ("run-2", &second)] {
        let delivered = deliver(
            &file,
            WriteMode::Upsert,
            own.clone(),
            copy_of(run, 1),
            &["products", "reviews"],
            run_file,
            true,
        );
        assert_eq!(delivered, Status::Complete);
    }

    let mut held: Vec<(String, u64, String)> = held(&file)
        .into_iter()
        .map(|(_, dataset, record)| {
            (
                dataset,
                record["price"].as_u64().unwrap(),
                record["author"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    held.sort_by(|left, right| (&left.0, &left.2).cmp(&(&right.0, &right.2)));
    let expected: Vec<(String, u64, String)> = expected
        .iter()
        .map(|&(dataset, price, author)| (dataset.to_owned(), price, author.to_owned()))
        .collect();
    assert_eq!(held, expected);
}

/// A Record of a Dataset with no schema that lacks a field of its key
/// fails the keyed Sink with `rejected_records`, before anything of its
/// batch is written.
///
/// Covers: story:6-286
#[test]
fn a_record_without_its_key_fails_a_keyed_sink() {
    let (_folder, file) = destination();
    let run_file = declared(&[("products", &["sku"])]);
    commit_at(
        &run_file,
        "products",
        1,
        &[json!({"sku": "a", "price": 1}), json!({"price": 2})],
        FIRST,
    );

    let delivered = deliver(
        &file,
        WriteMode::Upsert,
        None,
        copy_of("run-1", 1),
        &["products"],
        &run_file,
        false,
    );

    assert_eq!(
        delivered,
        Status::Failed(Failure::new(FailureKind::RejectedRecords, ""))
    );
    assert_eq!(held(&file), Vec::<(String, String, Value)>::new());
}

/// Under `history`, a copy of a Run that takes over from an older copy
/// removes the versions the older one wrote, and the version one of them
/// closed is current again.
///
/// Covers: story:6-286
#[test]
fn a_takeover_under_history_reopens_what_the_older_copy_closed() {
    let (_folder, file) = destination();
    let earlier = declared(&[("products", &["sku"])]);
    commit_at(
        &earlier,
        "products",
        1,
        &[json!({"sku": "a", "price": 0})],
        FIRST,
    );
    deliver(
        &file,
        WriteMode::History,
        None,
        copy_of("run-0", 1),
        &["products"],
        &earlier,
        true,
    );
    let older = declared(&[("products", &["sku"])]);
    commit_at(
        &older,
        "products",
        1,
        &[json!({"sku": "a", "price": 1})],
        SECOND,
    );
    deliver(
        &file,
        WriteMode::History,
        None,
        copy_of("run-1", 1),
        &["products"],
        &older,
        false,
    );
    // The later copy's run file lost the older's Record of `a`, as a power
    // cut before its commit loses it.
    let later = declared(&[("products", &["sku"])]);
    commit_at(
        &later,
        "products",
        1,
        &[json!({"sku": "b", "price": 2})],
        SECOND,
    );

    let delivered = deliver(
        &file,
        WriteMode::History,
        None,
        copy_of("run-1", 2),
        &["products"],
        &later,
        true,
    );

    assert_eq!(delivered, Status::Complete);
    assert_eq!(
        versions(&file),
        [
            (
                json!({"sku": "a", "price": 0}),
                r#"["a"]"#.to_owned(),
                FIRST.to_owned(),
                None
            ),
            (
                json!({"sku": "b", "price": 2}),
                r#"["b"]"#.to_owned(),
                SECOND.to_owned(),
                None
            ),
        ]
    );
}
