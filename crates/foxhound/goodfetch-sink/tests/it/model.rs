//! Delivery under the crash VFS (ADR 0060), judged by the fencing checker
//! (issue 340) for a Sink of each Guarantee class (ADR 0026): the real
//! SQLite Sink, Fenced, and a model destination of each other class. The
//! run file crashes inside a batch, a later copy of the Run recovers it and
//! delivers, and a zombie copy delivers beside it; the Observation log
//! holds each fence, commit, write and the rows read back.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use goodfetch_sink::{
    Batch, DeliveryId, Driver, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy,
    SqliteSink, Status, Written,
};
use goodfetch_test_support::{
    CrashKind, CrashVfs, Fault, ObservationLog, Operation, Record, TestEnv, ValueDigest,
    after_crash, fencing,
};
use rstest::rstest;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::run_files::{TARGET, commit, copy, items, rows};

/// The Sink's name in every case.
const SINK: &str = "warehouse";

/// The Dataset every case delivers.
const DATASET: &str = "products";

/// The run file's path on the crash VFS's disk.
const RUN_FILE: &str = "run-file.sqlite";

/// The case's Observation log, which every copy's Sink writes to.
type Log = Rc<RefCell<ObservationLog<()>>>;

/// A model destination's rows, shared by every copy's handle on it.
type Held = Rc<RefCell<Vec<Row>>>;

/// One row a model destination holds.
struct Row {
    run_epoch: u64,
    key: String,
    number: u64,
    value: Value,
}

/// Where a case's Records land.
enum Destination {
    /// The SQLite Sink's database file.
    Sqlite(PathBuf),
    /// A model destination of the class, in memory.
    Model(GuaranteeClass, Held),
}

impl Destination {
    /// A destination of `class`, a file in the test's folder for the
    /// SQLite Sink.
    fn of(class: GuaranteeClass, env: &TestEnv) -> Self {
        match class {
            GuaranteeClass::Fenced => Self::Sqlite(env.folder().join("destination.sqlite")),
            other => Self::Model(other, Held::default()),
        }
    }

    /// A new handle on the destination, as a copy of the Run opens one,
    /// logging each write to `log`.
    fn sink(&self, log: &Log) -> Logged {
        let handle = match self {
            Self::Sqlite(path) => Handle::Sqlite(Box::new(SqliteSink::new(
                Connection::open(path).unwrap(),
                TARGET,
            ))),
            Self::Model(class, held) => Handle::Model(*class, Rc::clone(held)),
        };
        Logged {
            handle,
            log: Rc::clone(log),
        }
    }

    /// Each row the destination holds: its run epoch, key and value.
    fn visible(&self) -> Vec<(u64, String, Value)> {
        match self {
            Self::Sqlite(path) => rows(path)
                .into_iter()
                .map(|row| (row.run_epoch, row.delivery_id, row.record))
                .collect(),
            Self::Model(_, held) => held
                .borrow()
                .iter()
                .map(|row| (row.run_epoch, row.key.clone(), row.value.clone()))
                .collect(),
        }
    }
}

/// One copy's handle on a destination.
enum Handle {
    Sqlite(Box<SqliteSink>),
    Model(GuaranteeClass, Held),
}

/// A copy's Sink, logging each Record it writes before the write.
struct Logged {
    handle: Handle,
    log: Log,
}

impl Primitives for Logged {
    type Error = rusqlite::Error;

    fn probe(&mut self, run: &RunCopy) -> rusqlite::Result<Probe> {
        match &mut self.handle {
            Handle::Sqlite(sink) => sink.probe(run),
            Handle::Model(class, held) => {
                let mut delivered = BTreeMap::new();
                for row in held
                    .borrow()
                    .iter()
                    .filter(|row| row.run_epoch == run.run_epoch)
                {
                    let last = delivered.entry(DATASET.to_owned()).or_insert(0);
                    *last = row.number.max(*last);
                }
                Ok(Probe {
                    class: *class,
                    delivered,
                })
            }
        }
    }

    fn write(&mut self, batch: &Batch) -> rusqlite::Result<Written> {
        let run_epoch = batch.run.run_epoch;
        for record in &batch.records {
            self.log.borrow_mut().push(Record::new(
                format!("copy-{run_epoch}"),
                Operation::Write {
                    epoch: run_epoch,
                    key: record.delivery_id.as_str().to_owned(),
                    digest: digest(&record.value),
                },
            ));
        }
        match &mut self.handle {
            Handle::Sqlite(sink) => sink.write(batch),
            Handle::Model(class, held) => {
                let mut held = held.borrow_mut();
                for record in &batch.records {
                    let row = Row {
                        run_epoch,
                        key: record.delivery_id.as_str().to_owned(),
                        number: record.number,
                        value: record.value.clone(),
                    };
                    let existing = held.iter_mut().find(|held| held.key == row.key);
                    match (*class, existing) {
                        (GuaranteeClass::AtLeastOnce, _) | (_, None) => held.push(row),
                        (GuaranteeClass::RecordFenced, Some(existing))
                            if existing.run_epoch > run_epoch => {}
                        (_, Some(existing)) => *existing = row,
                    }
                }
                Ok(Written::Delivered)
            }
        }
    }

    fn map_failure(&self, error: rusqlite::Error) -> Failure {
        Failure::new(FailureKind::Transient, error.to_string())
    }
}

/// The digest the log holds for `value`.
fn digest(value: &Value) -> ValueDigest {
    ValueDigest::of(value.to_string())
}

/// The class the fencing checker judges a Sink of `class` by.
fn judged(class: GuaranteeClass) -> goodfetch_test_support::GuaranteeClass {
    match class {
        GuaranteeClass::Fenced => goodfetch_test_support::GuaranteeClass::Fenced,
        GuaranteeClass::RecordFenced => goodfetch_test_support::GuaranteeClass::RecordFenced,
        GuaranteeClass::Keyed => goodfetch_test_support::GuaranteeClass::Keyed,
        GuaranteeClass::AtLeastOnce => goodfetch_test_support::GuaranteeClass::AtLeastOnce,
    }
}

/// A driver for the copy `run_epoch` of `run-1`, three Records a batch.
fn driver(sink: Logged, run_epoch: u64) -> Driver<Logged> {
    Driver::new(SINK, sink, copy(run_epoch), vec![DATASET.to_owned()]).with_batch_records(3)
}

/// Opens the run file on the crash VFS, synced at each commit as the run
/// state's default `batch` level syncs.
fn open(vfs: &CrashVfs, create: bool) -> Connection {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags_and_vfs(RUN_FILE, flags, vfs.name()).unwrap();
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;")
        .unwrap();
    connection
}

/// Commits `values` to the run file as one batch, numbered from `first`,
/// and logs each Record's commit once the batch is written.
fn commit_batch(run_file: &Connection, log: &Log, first: u64, values: &[Value]) {
    let batch = run_file.unchecked_transaction().unwrap();
    commit(&batch, DATASET, i64::try_from(first).unwrap(), values);
    batch.commit().unwrap();
    for (number, value) in (first..).zip(values) {
        log.borrow_mut().push(Record::new(
            "run-file",
            Operation::Commit {
                key: DeliveryId::new("run-1", SINK, DATASET, number)
                    .as_str()
                    .to_owned(),
                digest: digest(value),
            },
        ));
    }
}

/// The digests of every value the log shows written.
fn written(log: &Log) -> Vec<ValueDigest> {
    log.borrow()
        .records()
        .iter()
        .filter_map(|record| match &record.operation {
            Operation::Write { digest, .. } => Some(digest.clone()),
            _ => None,
        })
        .collect()
}

/// A Run's file crashes inside a batch: the copy before the crash
/// delivered only committed batches, never the one being written, and
/// delivers nothing more once crashed; the copy that recovers the file
/// delivers under the next run epoch while a zombie of the first copy
/// delivers beside it; and the destination, read back, passes the fencing
/// checker for the class its Sink declares: every committed Record
/// visible, and nothing the class forbids from the zombie.
///
/// Covers: story:6-285
#[rstest]
fn delivery_across_a_crash_inside_a_batch_passes_the_fencing_checker(
    #[values(
        GuaranteeClass::Fenced,
        GuaranteeClass::RecordFenced,
        GuaranteeClass::Keyed,
        GuaranteeClass::AtLeastOnce
    )]
    class: GuaranteeClass,
    #[values(CrashKind::Process, CrashKind::PowerCut)] kind: CrashKind,
) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("sink-model"), env.seed()).unwrap();
    let log: Log = Rc::new(RefCell::new(ObservationLog::for_test(&env)));
    let destination = Destination::of(class, &env);
    let writer = open(&vfs, true);
    for statement in goodfetch_dataset::TABLES.statements {
        writer.execute_batch(statement).unwrap();
    }
    let reader = open(&vfs, false);

    log.borrow_mut()
        .push(Record::new("copy-1", Operation::Fence { epoch: 1 }));
    let mut first = driver(destination.sink(&log), 1);
    commit_batch(&writer, &log, 1, &items("first-", 4));
    assert_eq!(first.deliver(&reader).unwrap(), Status::Complete);
    commit_batch(&writer, &log, 5, &items("second-", 4));
    let lost = items("lost-", 3);
    let open_batch = writer.unchecked_transaction().unwrap();
    commit(&open_batch, DATASET, 9, &lost);
    assert_eq!(first.deliver(&reader).unwrap(), Status::Complete);
    let written_before_the_crash = written(&log);
    assert!(
        lost.iter()
            .all(|value| !written_before_the_crash.contains(&digest(value))),
        "a Record of the batch being written was delivered"
    );

    vfs.crash(kind);
    log.borrow_mut().push(Record::new(
        "engine",
        Operation::Fault(match kind {
            CrashKind::Process => Fault::Crash,
            CrashKind::PowerCut => Fault::PowerCut,
        }),
    ));
    drop(open_batch);
    assert!(
        first.deliver(&reader).is_err(),
        "the crashed copy delivered"
    );

    let recovered = after_crash(&vfs, RUN_FILE).unwrap();
    log.borrow_mut()
        .push(Record::new("copy-2", Operation::Fence { epoch: 2 }));
    let mut current = driver(destination.sink(&log), 2);
    assert_eq!(current.deliver(&recovered).unwrap(), Status::Complete);
    commit_batch(&recovered, &log, 9, &items("after-", 3));
    let mut zombie = driver(destination.sink(&log), 1);
    let zombie_status = zombie.deliver(&recovered).unwrap();
    assert_eq!(current.deliver(&recovered).unwrap(), Status::Complete);
    for (run_epoch, key, value) in destination.visible() {
        log.borrow_mut().push(Record::new(
            "destination",
            Operation::Visible {
                epoch: run_epoch,
                key,
                digest: digest(&value),
            },
        ));
    }

    assert_eq!(current.class(), Some(class));
    assert_eq!(
        zombie_status,
        match class {
            GuaranteeClass::Fenced => Status::Superseded { run_epoch: 2 },
            _ => Status::Complete,
        }
    );
    fencing(&log.borrow(), judged(class)).unwrap();
}
