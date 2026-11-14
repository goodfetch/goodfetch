//! The writer's open batch at a level that batches attempts (ADR 0142):
//! an attempt's write is answered before its batch is written, a barrier,
//! a threshold, the interval or the writer's end writes the batch, and a
//! batch costs one transaction and one sync, all or nothing.

use std::num::NonZeroU64;
use std::time::Duration;

use goodfetch_counted_work::Row;
use goodfetch_protocol::run::durability::BatchThreshold;
use goodfetch_test_support::{CountingVfs, CrashKind, CrashVfs, TestEnv, after_crash};

use super::{insert, items, run_row, with_writer_options};
use crate::run_state::{Durability, RunState, RunStateOptions, Syncing};

/// An attempt's write of `value` that ends one attempt.
async fn ended(state: &RunState, value: i64) {
    state
        .write_with(Syncing::Attempt { ended: 1, bytes: 0 }, insert(value))
        .await
        .unwrap();
}

fn at(value: u64) -> BatchThreshold {
    BatchThreshold::At(NonZeroU64::new(value).unwrap())
}

/// `batch`'s preset with only the thresholds given.
fn batching(pages: BatchThreshold, interval: BatchThreshold, bytes: BatchThreshold) -> Durability {
    Durability {
        pages,
        interval,
        bytes,
        ..Durability::BATCH
    }
}

fn options(durability: Durability) -> RunStateOptions {
    RunStateOptions {
        durability,
        ..RunStateOptions::default()
    }
}

/// Waits until the writer acknowledged more commits than `before`.
async fn committed_after(state: &RunState, before: u64) {
    state
        .commits()
        .wait_for(|count| *count > before)
        .await
        .unwrap();
}

/// At `batch`, an attempt's write is answered once it is in the open batch,
/// which a reader does not see; a barrier writes the batch with its own
/// write before it answers.
///
/// Covers: story:3-25, setting:run_state.durability
#[test]
fn at_batch_an_attempts_write_is_answered_before_a_barrier_writes_its_batch() {
    let env = TestEnv::new();

    let (before, after) = with_writer_options(&env, &options(Durability::BATCH), async |state| {
        ended(state, 1).await;
        let before = state.read(items).await.unwrap();
        state.write(insert(2)).await.unwrap();
        (before, state.read(items).await.unwrap())
    });

    assert_eq!((before, after), (vec![], vec![1, 2]));
}

/// A seal writes the open batch and answers once it is in the file, so a
/// read that follows it sees every write answered before it; with no batch
/// open it commits nothing.
///
/// Covers: story:3-25
#[test]
fn a_seal_writes_the_open_batch_and_with_none_open_commits_nothing() {
    let env = TestEnv::new();
    let options = options(Durability::BATCH);
    let work = options.work.clone();

    let (sealed, transactions) = with_writer_options(&env, &options, async |state| {
        ended(state, 1).await;
        state.seal().await.unwrap();
        let sealed = state.read(items).await.unwrap();
        let before = work.read(Row::RunTransactions);
        state.seal().await.unwrap();
        (sealed, work.read(Row::RunTransactions) - before)
    });

    assert_eq!((sealed, transactions), (vec![1], 0));
}

/// At `full`, each attempt's writes are a batch of their own: an attempt's
/// write is synced and in the file as it is answered.
///
/// Covers: story:3-25, setting:run_state.durability
#[test]
fn at_full_an_attempts_write_is_synced_and_written_as_it_is_answered() {
    let env = TestEnv::new();
    let vfs = CountingVfs::register();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        ..options(Durability::FULL)
    };

    let (synced, found) = with_writer_options(&env, &options, async |state| {
        state.write(run_row).await.unwrap();
        let before = vfs.syncs();
        ended(state, 1).await;
        (vfs.syncs() - before, state.read(items).await.unwrap())
    });

    assert_eq!((synced, found), (1, vec![1]));
}

/// The attempts a batch ended, the bytes of its rows and its age each seal
/// it at their threshold: the writer writes it without a barrier.
///
/// Covers: setting:run_state.batch.pages, setting:run_state.batch.bytes,
/// setting:run_state.batch.interval
#[rstest::rstest]
#[case::pages(batching(at(3), BatchThreshold::Never, BatchThreshold::Never), Syncing::Attempt { ended: 1, bytes: 0 }, 3)]
#[case::bytes(batching(BatchThreshold::Never, BatchThreshold::Never, at(100)), Syncing::Attempt { ended: 0, bytes: 40 }, 3)]
fn a_threshold_writes_the_batch_it_reaches(
    #[case] durability: Durability,
    #[case] syncing: Syncing,
    #[case] writes: i64,
) {
    let env = TestEnv::new();

    let (short, reached) = with_writer_options(&env, &options(durability), async |state| {
        let before = *state.commits().borrow();
        for value in 1..writes {
            state.write_with(syncing, insert(value)).await.unwrap();
        }
        let short = state.read(items).await.unwrap();
        state.write_with(syncing, insert(writes)).await.unwrap();
        committed_after(state, before).await;
        (short, state.read(items).await.unwrap())
    });

    assert_eq!((short, reached), (vec![], vec![1, 2, 3]));
}

/// The interval writes an idle writer's open batch once it passed since
/// the batch opened.
///
/// Covers: setting:run_state.batch.interval
#[test]
fn the_interval_writes_an_idle_writers_open_batch() {
    let env = TestEnv::new();
    let interval = Duration::from_millis(1_000);
    let durability = batching(BatchThreshold::Never, at(1_000), BatchThreshold::Never);

    let (early, late) = with_writer_options(&env, &options(durability), async |state| {
        ended(state, 1).await;
        tokio::time::sleep(interval / 2).await;
        let early = state.read(items).await.unwrap();
        tokio::time::sleep(interval).await;
        (early, state.read(items).await.unwrap())
    });

    assert_eq!((early, late), (vec![], vec![1]));
}

/// A writer whose every handle is gone writes its open batch as it ends,
/// so the file holds it when it opens again.
///
/// Covers: story:3-25
#[test]
fn a_writer_whose_handles_are_gone_writes_its_open_batch() {
    let env = TestEnv::new();
    let durability = batching(
        BatchThreshold::Never,
        BatchThreshold::Never,
        BatchThreshold::Never,
    );
    with_writer_options(&env, &options(durability), async |state| {
        ended(state, 1).await;
        ended(state, 2).await;
    });

    let reopened = with_writer_options(&env, &options(durability), async |state| {
        state.read(items).await.unwrap()
    });

    assert_eq!(reopened, vec![1, 2]);
}

/// A batch is one transaction and one sync, however many attempts it
/// holds, at `batch` (`run_state.batch.sync: each`).
///
/// Covers: story:3-25, setting:run_state.durability
#[test]
fn a_batch_of_attempts_and_its_barrier_commit_in_one_synced_transaction() {
    let env = TestEnv::new();
    let vfs = CountingVfs::register();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        ..options(Durability::BATCH)
    };
    let work = options.work.clone();

    let (transactions, syncs) = with_writer_options(&env, &options, async |state| {
        state.write(run_row).await.unwrap();
        let (transactions, syncs) = (work.read(Row::RunTransactions), vfs.syncs());
        for value in 1..=5 {
            ended(state, value).await;
        }
        state.write(insert(6)).await.unwrap();
        (
            work.read(Row::RunTransactions) - transactions,
            vfs.syncs() - syncs,
        )
    });

    assert_eq!((transactions, syncs), (1, 1));
}

/// A batch is written all or nothing: a power cut before its commit leaves
/// the run file as of the batch before, though its attempts were answered.
///
/// Covers: story:3-25
#[test]
fn a_power_cut_inside_an_open_batch_leaves_the_file_as_of_the_batch_before() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("open-batch"), env.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        ..options(Durability::BATCH)
    };
    let path = env.folder().join("run.sqlite");

    with_writer_options(&env, &options, async |state| {
        state.write(insert(1)).await.unwrap();
        ended(state, 2).await;
        vfs.crash(CrashKind::PowerCut);
    });
    let after = after_crash(&vfs, &path.to_string_lossy()).unwrap();

    assert_eq!(items(&after).unwrap(), vec![1]);
}

/// At `off`, the writer's last write rides the batch open when the writer
/// stops, which its stop writes as one batch; the sync that clears the
/// "unclean" mark then writes nothing of the batch's.
///
/// Covers: story:3-27, setting:run_state.durability
#[test]
fn at_off_the_last_write_rides_the_open_batch_its_stop_writes() {
    let env = TestEnv::new();
    let options = options(Durability::OFF);
    let work = options.work.clone();

    let batches = with_writer_options(&env, &options, async |state| {
        let before = work.read(Row::RunBatches);
        ended(state, 1).await;
        state.write_last(insert(2)).await.unwrap();
        work.read(Row::RunBatches) - before
    });

    assert_eq!(batches, 1);
}
