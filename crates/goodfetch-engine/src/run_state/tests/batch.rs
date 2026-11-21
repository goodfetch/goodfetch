//! Run-state batches through the writer's interface (ADR 0142): a sealed
//! batch written all or nothing, and one sealed batch waiting at a time.

use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use rusqlite::Connection;

use super::{ITEMS, insert, items, with_writer, with_writer_options};
use crate::run_state::{
    BatchRows, BatchWriter, Durability, RunState, RunStateError, RunStateOptions, SealedBatch,
    Syncing,
};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Rows of the tests' table, decided before they are sealed: `None` is a
/// row its `NOT NULL` column refuses.
struct Items(Vec<Option<i64>>);

impl BatchRows for Items {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        let mut insert = writer.statement("INSERT INTO items (value) VALUES (?1)")?;
        for value in self.0 {
            insert.write([value])?;
        }
        Ok(())
    }
}

/// A sealed batch of `values`, which no sync of its own follows.
async fn sealed(state: &RunState, values: &[i64]) -> Result<(), RunStateError> {
    let rows = Items(values.iter().copied().map(Some).collect());
    state
        .write_batch(Syncing::Deferred { charged: 0 }, SealedBatch::of(rows))
        .await
}

/// The items in the order they were written.
fn in_order(connection: &Connection) -> rusqlite::Result<Vec<i64>> {
    connection
        .prepare("SELECT value FROM items ORDER BY rowid")?
        .query_map([], |row| row.get(0))?
        .collect()
}

/// Rows that crash the process once the first of them is written.
struct CutOff {
    vfs: CrashVfs,
    values: Vec<i64>,
}

impl BatchRows for CutOff {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        let mut insert = writer.statement("INSERT INTO items (value) VALUES (?1)")?;
        for (written, value) in self.values.into_iter().enumerate() {
            if written == 1 {
                self.vfs.crash(CrashKind::Process);
            }
            insert.write([value])?;
        }
        Ok(())
    }
}

/// A sealed batch is written all or nothing (ADR 0142): a crash inside it
/// leaves the run file as of the batch before, with none of its rows, at
/// `full`, where the batch before was written as it was answered.
///
/// Covers: story:3-25
#[test]
fn a_crash_inside_a_sealed_batch_leaves_the_run_file_as_of_the_batch_before() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("batch-crash"), env.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        durability: Durability::FULL,
        ..RunStateOptions::default()
    };
    let path = env.folder().join("run.sqlite");

    let cut = with_writer_options(&env, &options, async |state| {
        sealed(state, &[1, 2]).await.unwrap();
        let rows = CutOff {
            vfs: vfs.clone(),
            values: vec![3, 4, 5],
        };
        state
            .write_batch(Syncing::Required, SealedBatch::of(rows))
            .await
    });
    let after = after_crash(&vfs, &path.to_string_lossy()).unwrap();

    assert_eq!((cut.is_err(), items(&after).unwrap()), (true, vec![1, 2]));
}

/// A sealed batch one of whose rows the file refuses writes none of its
/// rows, and the writes beside it commit: the read, a write, commits them
/// at every level.
#[test]
fn a_sealed_batch_with_a_refused_row_writes_none_of_its_rows() {
    let env = TestEnv::new();

    let (refused, committed) = with_writer(&env, 1, async |state| {
        let refused = state
            .write_batch(
                Syncing::Deferred { charged: 0 },
                SealedBatch::of(Items(vec![Some(1), None, Some(3)])),
            )
            .await;
        sealed(state, &[4]).await.unwrap();
        (refused, state.write(items).await.unwrap())
    });

    assert_eq!(
        (
            matches!(refused, Err(RunStateError::Sqlite { .. })),
            committed
        ),
        (true, vec![4]),
        "{refused:?}"
    );
}

/// Rows whose write waits for the test to release it.
struct Held {
    value: i64,
    started: tokio::sync::oneshot::Sender<()>,
    released: std::sync::mpsc::Receiver<()>,
}

impl BatchRows for Held {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        writer
            .statement("INSERT INTO items (value) VALUES (?1)")?
            .write([self.value])?;
        let _ = self.started.send(());
        let _ = self.released.recv();
        Ok(())
    }
}

/// One sealed batch waits at a time (ADR 0142): while the writer writes a
/// batch, a second sealed batch is not queued, so a write queued after it
/// was sealed is written before it. The read, a write, commits them at
/// every level.
#[test]
fn a_second_sealed_batch_waits_while_the_first_is_written() {
    let env = TestEnv::new();
    let path = env.folder().join("run.sqlite");

    let (waited, order) = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Threads);
        let state = RunState::open(path, &RunStateOptions::default(), vec![ITEMS], 1, &owner)
            .await
            .unwrap();
        let (started, starts) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let first = state.write_batch(
            Syncing::Deferred { charged: 0 },
            SealedBatch::of(Held {
                value: 1,
                started,
                released,
            }),
        );
        let rest = async {
            starts.await.unwrap();
            let second = sealed(&state, &[2]);
            tokio::pin!(second);
            let waited = std::future::poll_fn(|context| {
                std::task::Poll::Ready(second.as_mut().poll(context).is_pending())
            })
            .await;
            let beside = state.reserve().await.unwrap().write(insert(9));
            release.send(()).unwrap();
            second.await.unwrap();
            beside.await.unwrap();
            waited
        };
        let (first, waited) = tokio::join!(first, rest);
        first.unwrap();
        let order = state.write(in_order).await.unwrap();
        drop(state);
        owner.wait().await;
        (waited, order)
    });

    assert_eq!((waited, order), (true, vec![1, 9, 2]));
}
