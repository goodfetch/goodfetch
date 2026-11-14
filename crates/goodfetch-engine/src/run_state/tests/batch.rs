//! Run-state batches through the writer's interface (ADR 0142): a sealed
//! batch written all or nothing, one sealed batch waiting at a time, and a
//! barrier, which commits the batch in progress before it answers.

use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use rusqlite::Connection;

use super::{ITEMS, insert, items, with_counted_writer, with_writer, with_writer_options};
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
/// leaves the run file as of the batch before, with none of its rows.
///
/// Covers: story:3-25
#[test]
fn a_crash_inside_a_sealed_batch_leaves_the_run_file_as_of_the_batch_before() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("batch-crash"), env.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
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
/// rows, and the writes beside it commit.
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
        (refused, state.read(items).await.unwrap())
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
/// was sealed commits before it.
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
        let order = state.read(in_order).await.unwrap();
        drop(state);
        owner.wait().await;
        (waited, order)
    });

    assert_eq!((waited, order), (true, vec![1, 9, 2]));
}

/// A barrier commits the batch in progress before it answers (ADR 0142):
/// whoever holds that batch seals it when asked, the barrier's own write
/// finds its rows in the file, and the barrier is synced as the level says,
/// once at `full` (`run_state.batch.sync: each`) and never at `fast`
/// (`never`).
///
/// Covers: story:3-25, story:3-27, setting:run_state.durability
#[rstest::rstest]
#[case::each(Durability::FULL, 1)]
#[case::never(Durability::FAST, 0)]
fn a_barrier_commits_the_batch_in_progress_before_it_answers_synced_as_the_level_says(
    #[case] durability: Durability,
    #[case] syncs: u64,
) {
    let (found, added, committed) = with_counted_writer(durability, async |state, vfs| {
        let mut seals = state.seals().unwrap();
        let before = vfs.syncs();
        let sealing = async {
            let request = seals.next().await.unwrap();
            sealed(state, &[1, 2]).await.unwrap();
            request.sealed();
        };
        let barrier = state.barrier(|connection| {
            let found = in_order(connection)?;
            insert(3)(connection)?;
            Ok(found)
        });
        let (found, ()) = tokio::join!(barrier, sealing);
        let added = vfs.syncs() - before;
        (found.unwrap(), added, state.read(in_order).await.unwrap())
    });

    assert_eq!(
        (found, added, committed),
        (vec![1, 2], syncs, vec![1, 2, 3])
    );
}

/// A barrier waits for no one when no batch is in progress: on a run file
/// whose batches no one seals, and once whoever sealed them is gone, it
/// commits its write as any other.
#[test]
fn a_barrier_with_no_batch_in_progress_commits_its_write() {
    let env = TestEnv::new();

    let committed = with_writer(&env, 1, async |state| {
        state.barrier(insert(1)).await.unwrap();
        let seals = state.seals().unwrap();
        let taken = state.seals().is_none();
        drop(seals);
        state.barrier(insert(2)).await.unwrap();
        (taken, state.read(items).await.unwrap())
    });

    assert_eq!(committed, (true, vec![1, 2]));
}
