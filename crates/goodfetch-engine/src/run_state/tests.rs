//! `run_state` through its interface, on real SQLite in the test's folder or
//! through the crash VFS (spec #213's Seam 4): durability pragmas, grouped
//! commits under the run epoch, format refusals, and a model-based test of
//! crashes, power cuts and reopening against a reference model (ADR 0060).

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod batch;
mod batching;
mod model;
mod wal;

use std::time::Duration;

use goodfetch_counted_work::Row;
use goodfetch_test_support::{CrashKind, CrashVfs, IoFault, TestEnv};
use rusqlite::Connection;

use super::{
    BatchEnd, Charges, DEFERRED_CHARGES, Durability, PAGE_CACHE_KIB, RunState, RunStateError,
    RunStateOptions, RunStateTables, Syncing, WriteWatch, open_reader, unclean,
};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A contributed table the tests write through the writer.
pub(super) const ITEMS: RunStateTables = RunStateTables {
    name: "test",
    statements: &["CREATE TABLE IF NOT EXISTS items (value INTEGER NOT NULL) STRICT"],
};

pub(super) fn insert(
    value: i64,
) -> impl FnOnce(&Connection) -> rusqlite::Result<()> + Send + 'static {
    move |connection| {
        connection
            .execute("INSERT INTO items (value) VALUES (?1)", [value])
            .map(drop)
    }
}

pub(super) fn items(connection: &Connection) -> rusqlite::Result<Vec<i64>> {
    let mut statement = connection.prepare("SELECT value FROM items ORDER BY value")?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()
}

fn pragma(name: &'static str) -> impl FnOnce(&Connection) -> rusqlite::Result<String> + Send {
    move |connection| {
        connection.pragma_query_value(None, name, |row| {
            row.get::<_, rusqlite::types::Value>(0)
                .map(|value| match value {
                    rusqlite::types::Value::Integer(number) => number.to_string(),
                    rusqlite::types::Value::Text(text) => text,
                    other => format!("{other:?}"),
                })
        })
    }
}

/// Opens `file` in the test's folder, runs `test` on its writer, then
/// stops the writer.
fn with_writer<T>(env: &TestEnv, epoch: u64, test: impl AsyncFnOnce(&RunState) -> T) -> T {
    with_writer_at(env, &RunStateOptions::default(), epoch, test)
}

/// As [`with_writer`], the file opened with `options` at run epoch 1.
fn with_writer_options<T>(
    env: &TestEnv,
    options: &RunStateOptions,
    test: impl AsyncFnOnce(&RunState) -> T,
) -> T {
    with_writer_at(env, options, 1, test)
}

fn with_writer_at<T>(
    env: &TestEnv,
    options: &RunStateOptions,
    epoch: u64,
    test: impl AsyncFnOnce(&RunState) -> T,
) -> T {
    let path = env.folder().join("run.sqlite");
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(path, options, vec![ITEMS], epoch, &owner)
            .await
            .unwrap();
        let answer = test(&state).await;
        drop(state);
        owner.wait().await;
        answer
    })
}

/// Covers: story:3-25, story:3-26
#[test]
fn a_run_file_opens_in_wal_mode_with_full_synchronous_commits() {
    let env = TestEnv::new();

    let pragmas = with_writer(&env, 1, async |state| {
        (
            state.read(pragma("journal_mode")).await.unwrap(),
            state.write(pragma("synchronous")).await.unwrap(),
        )
    });

    assert_eq!(pragmas, ("wal".to_owned(), "2".to_owned()));
}

/// A writer opens a new run file another connection writes once that write
/// ends: its switch of the file to WAL waits, as SQLite answers that switch
/// busy at once (#1773's class).
///
/// Covers: story:3-25
#[test]
fn a_writer_opens_a_new_run_file_beside_another_write_of_it() {
    let env = TestEnv::new();
    let path = env.folder().join("run.sqlite");

    let opened = crate::sqlite_wal::tests::opened_beside_a_write(&path, || {
        goodfetch_test_support::runtime().block_on(async {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let opened = RunState::open(
                path.clone(),
                &RunStateOptions::default(),
                vec![ITEMS],
                1,
                &owner,
            )
            .await
            .map(drop);
            owner.wait().await;
            opened
        })
    });

    assert_eq!(opened, Ok(()));
}

/// `run_state.durability = fast` trades the power-cut guarantee for speed:
/// the file commits with `synchronous=NORMAL`, still in WAL mode.
///
/// Covers: story:3-27, setting:run_state.durability
#[test]
fn a_run_file_in_fast_durability_commits_with_normal_synchronous() {
    let env = TestEnv::new();
    let options = RunStateOptions {
        durability: Durability::FAST,
        ..RunStateOptions::default()
    };

    let pragmas = with_writer_options(&env, &options, async |state| {
        (
            state.read(pragma("journal_mode")).await.unwrap(),
            state.write(pragma("synchronous")).await.unwrap(),
        )
    });

    assert_eq!(pragmas, ("wal".to_owned(), "1".to_owned()));
}

/// `run_state.durability = off` syncs nothing but its "unclean" mark (ADR
/// 0138): the writer commits with `synchronous=OFF`, the file holds the mark
/// while the writer runs, and its clean stop, the writer's last write,
/// clears it.
///
/// Covers: story:3-27, setting:run_state.durability
#[test]
fn a_run_file_at_off_commits_unsynced_and_holds_its_unclean_mark_until_its_clean_stop() {
    let env = TestEnv::new();
    let options = RunStateOptions {
        durability: Durability::OFF,
        ..RunStateOptions::default()
    };
    let path = env.folder().join("run.sqlite");
    let marked = || unclean(&path, None, Durability::OFF).unwrap();

    let (synchronous, running) = with_writer_options(&env, &options, async |state| {
        let synchronous = state.write(pragma("synchronous")).await.unwrap();
        let running = marked();
        state.write_last(insert(1)).await.unwrap();
        (synchronous, running)
    });

    assert_eq!(
        (synchronous.as_str(), running, marked()),
        ("0", true, false)
    );
}

/// A writer at `off` that ends without its clean stop, as a crash ends it,
/// leaves the mark; a file below `off` never holds it (ADR 0138).
///
/// Covers: story:3-27
#[rstest::rstest]
#[case::off(Durability::OFF, true)]
#[case::full(Durability::FULL, false)]
#[case::fast(Durability::FAST, false)]
fn a_writer_that_ends_without_its_clean_stop_leaves_the_mark_only_at_off(
    #[case] durability: Durability,
    #[case] left: bool,
) {
    let env = TestEnv::new();
    let options = RunStateOptions {
        durability,
        ..RunStateOptions::default()
    };

    with_writer_options(&env, &options, async |state| {
        state.write(insert(1)).await.unwrap();
    });

    assert_eq!(
        unclean(&env.folder().join("run.sqlite"), None, durability),
        Ok(left)
    );
}

/// `run_state.durability = periodic` commits as `fast` does, and syncs one
/// group once `run_state.commit_interval` passed: an idle writer's timer
/// syncs what it committed, and the next write commits unsynced again
/// (ADR 0138).
///
/// Covers: story:3-27, setting:run_state.durability, setting:run_state.commit_interval
#[test]
fn at_periodic_commits_sync_once_the_interval_passed_even_with_the_writer_idle() {
    let interval = Duration::from_secs(5);
    let added = with_counted_writer(Durability::periodic(interval), async |state, vfs| {
        let mut added = Vec::new();
        for value in [1, 2] {
            let before = vfs.syncs();
            state.write(insert(value)).await.unwrap();
            added.push(vfs.syncs() - before);
        }
        let before = vfs.syncs();
        tokio::time::sleep(interval + interval / 2).await;
        added.push(vfs.syncs() - before);
        let before = vfs.syncs();
        state.write(insert(3)).await.unwrap();
        added.push(vfs.syncs() - before);
        added
    });

    assert_eq!(added, vec![0, 0, 1, 0]);
}

/// The writer's page cache is bounded at `PAGE_CACHE_KIB`, so a Frontier
/// of any size keeps its memory under ADR 0134's 8 bytes per URL at
/// 100,000 URLs, its visited set staying on disk.
#[test]
fn a_run_file_bounds_its_page_cache() {
    let env = TestEnv::new();

    let cache_size = with_writer(&env, 1, async |state| {
        state.write(pragma("cache_size")).await.unwrap()
    });

    assert_eq!(cache_size, format!("-{PAGE_CACHE_KIB}"));
}

/// ADR 0138: a savepoint's statement journal stays in memory, so none
/// spills to `$TMPDIR`.
#[test]
fn a_run_file_keeps_its_savepoint_journals_in_memory() {
    let env = TestEnv::new();

    let temp_store = with_writer(&env, 1, async |state| {
        state.write(pragma("temp_store")).await.unwrap()
    });

    assert_eq!(temp_store, "2");
}

/// The size of the file `name` in the test's folder, 0 when there is none.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads a file's size between its writer's steps, on its own runtime with nothing else to run"
)]
fn size_of(env: &TestEnv, name: &str) -> u64 {
    std::fs::metadata(env.folder().join(name)).map_or(0, |metadata| metadata.len())
}

/// ADR 0138: the writer runs no checkpoint of its own, so a WAL under the
/// checkpointer's threshold stays in the WAL, though it is past SQLite's
/// 1,000-page automatic checkpoint.
#[test]
fn the_writer_leaves_a_wal_under_the_threshold_uncheckpointed() {
    let env = TestEnv::new();

    let (before, after, wal) = with_writer(&env, 1, async |state| {
        let before = size_of(&env, "run.sqlite");
        state.write(blob(5)).await.unwrap();
        (
            before,
            size_of(&env, "run.sqlite"),
            size_of(&env, "run.sqlite-wal"),
        )
    });

    assert_eq!(after, before);
    assert!(wal >= 5 * 1_048_576, "{wal}");
}

/// ADR 0138: past its threshold of WAL, the checkpointer copies the WAL into
/// the file on its own connection, while the writer goes on.
#[test]
fn the_checkpointer_copies_a_wal_past_its_threshold_into_the_file() {
    let env = TestEnv::new();
    let options = RunStateOptions {
        wal_checkpoint_after: 1_048_576,
        ..RunStateOptions::default()
    };

    let copied = with_writer_options(&env, &options, async |state| {
        state.write(blob(2)).await.unwrap();
        for _ in 0..1_000 {
            if size_of(&env, "run.sqlite") >= 2 * 1_048_576 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        state.write(insert(1)).await.unwrap();
        size_of(&env, "run.sqlite")
    });

    assert!(copied >= 2 * 1_048_576, "{copied}");
}

/// A clean stop, the writer's last write, leaves the WAL as it is (#2117):
/// its commit synced the WAL, so no checkpoint copies it into the file, and
/// a reader that still holds the file open reads every write.
#[test]
fn a_clean_stop_leaves_the_wal_to_its_readers() {
    let env = TestEnv::new();
    let path = env.folder().join("run.sqlite");

    let reader = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            path.clone(),
            &RunStateOptions::default(),
            vec![ITEMS],
            1,
            &owner,
        )
        .await
        .unwrap();
        let reader = open_reader(&path, None).unwrap();
        items(&reader).unwrap();
        state.write(blob(1)).await.unwrap();
        state.write_last(insert(1)).await.unwrap();
        drop(state);
        owner.wait().await;
        reader
    });

    assert!(size_of(&env, "run.sqlite-wal") > 1_048_576);
    assert_eq!(items(&reader).unwrap(), vec![1]);
}

/// Covers: story:3-26
#[cfg(target_os = "macos")]
#[test]
fn a_run_file_syncs_with_fullfsync_on_macos() {
    let env = TestEnv::new();

    let fullfsync = with_writer(&env, 1, async |state| {
        (
            state.write(pragma("fullfsync")).await.unwrap(),
            state.write(pragma("checkpoint_fullfsync")).await.unwrap(),
        )
    });

    assert_eq!(fullfsync, ("1".to_owned(), "1".to_owned()));
}

/// Covers: format:run-state
#[test]
fn a_run_file_records_its_format_version() {
    let env = TestEnv::new();

    let format = with_writer(&env, 1, async |state| {
        state
            .read(|connection| {
                connection.query_row("SELECT name, version FROM format", [], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
            })
            .await
            .unwrap()
    });

    assert_eq!(format, ("run-state".to_owned(), "0.1".to_owned()));
}

/// Writes queued together commit together; one that fails rolls back only
/// its own changes, and the others are acknowledged.
#[test]
fn a_failing_write_commits_nothing_and_leaves_its_group_committed() {
    let env = TestEnv::new();

    let (answers, committed) = with_writer(&env, 1, async |state| {
        let failing = state.write(|connection| {
            insert(2)(connection)?;
            Err::<(), _>(rusqlite::Error::InvalidQuery)
        });
        let answers = tokio::join!(state.write(insert(1)), failing, state.write(insert(3)));
        let committed = state.read(items).await.unwrap();
        (
            (answers.0.is_ok(), answers.1.is_ok(), answers.2.is_ok()),
            committed,
        )
    });

    assert_eq!((answers, committed), ((true, false, true), vec![1, 3]));
}

/// What a watch of the writer's writes was told, in order.
#[derive(Debug, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "the test's watch collects what the writer's thread tells it, read once the writer stopped"
)]
struct Told(std::sync::Mutex<Vec<&'static str>>);

impl WriteWatch for Told {
    fn began(&self) {
        self.0.lock().unwrap().push("began");
    }

    fn failed(&self) {
        self.0.lock().unwrap().push("failed");
    }
}

/// A watch of the writer's writes is told as each write's work begins,
/// and when one fails, before the next begins, whether the writer commits
/// each group or batches attempts (#2169).
#[rstest::rstest]
#[case::full(Durability::FULL)]
#[case::batch(Durability::BATCH)]
fn a_watch_of_the_writes_is_told_as_each_begins_and_when_one_fails(#[case] durability: Durability) {
    let env = TestEnv::new();
    let options = RunStateOptions {
        durability,
        ..RunStateOptions::default()
    };
    let told = std::sync::Arc::new(Told::default());

    with_writer_options(&env, &options, async |state| {
        state.watch_writes(std::sync::Arc::clone(&told) as std::sync::Arc<dyn WriteWatch>);
        let failing = state.write(|connection| {
            insert(2)(connection)?;
            Err::<(), _>(rusqlite::Error::InvalidQuery)
        });
        let _answers = tokio::join!(state.write(insert(1)), failing, state.write(insert(3)));
    });

    assert_eq!(
        *told.0.lock().unwrap(),
        ["began", "began", "failed", "began"]
    );
}

/// What runs at the end of each batch in the tests: once armed, it writes
/// `1000 + n` at its `n`th run, and fails after writing when told to.
#[derive(Debug, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "the test's batch end, armed by the test and run on the writer's thread, one flag at a time"
)]
struct Ends {
    armed: std::sync::atomic::AtomicBool,
    failing: std::sync::atomic::AtomicBool,
    ran: std::sync::atomic::AtomicI64,
}

impl Ends {
    fn arm(&self, failing: bool) {
        self.failing
            .store(failing, std::sync::atomic::Ordering::SeqCst);
        self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl BatchEnd for Ends {
    fn waiting(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn ending(&self, connection: &Connection) -> rusqlite::Result<()> {
        self.armed.store(false, std::sync::atomic::Ordering::SeqCst);
        let ran = self.ran.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        insert(1000 + ran)(connection)?;
        if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    }
}

/// What a Run writes at the end of each batch rides that batch's commit,
/// whether the writer commits each group or batches attempts, as a write
/// of its own: told to the watch of the writes, and when it fails, its own
/// changes roll back and the batch commits without them (#2169).
#[rstest::rstest]
#[case::full(Durability::FULL)]
#[case::batch(Durability::BATCH)]
fn the_end_of_a_batch_rides_its_commit_and_fails_alone(#[case] durability: Durability) {
    let env = TestEnv::new();
    let options = RunStateOptions {
        durability,
        ..RunStateOptions::default()
    };
    let ends = std::sync::Arc::new(Ends::default());
    let told = std::sync::Arc::new(Told::default());

    let committed = with_writer_options(&env, &options, async |state| {
        state.watch_writes(std::sync::Arc::clone(&told) as std::sync::Arc<dyn WriteWatch>);
        state.end_batches_with(std::sync::Arc::clone(&ends) as std::sync::Arc<dyn BatchEnd>);
        ends.arm(false);
        state.write(insert(1)).await.unwrap();
        let first = state.read(items).await.unwrap();
        ends.arm(true);
        state.write(insert(2)).await.unwrap();
        let failed = state.read(items).await.unwrap();
        ends.arm(false);
        state.write(insert(3)).await.unwrap();
        let again = state.read(items).await.unwrap();
        (first, failed, again)
    });

    assert_eq!(
        (committed, told.0.lock().unwrap().clone()),
        (
            (vec![1, 1001], vec![1, 2, 1001], vec![1, 2, 3, 1001, 1003]),
            vec![
                "began", "began", "began", "began", "failed", "began", "began"
            ]
        )
    );
}

/// A write of one row of `mib` MiB, many times the writer's page cache, so
/// SQLite spills its pages to the WAL in the middle of its statement.
fn blob(mib: u32) -> impl FnOnce(&Connection) -> rusqlite::Result<()> + Send + 'static {
    move |connection| {
        connection.execute("CREATE TABLE IF NOT EXISTS blobs (data)", [])?;
        connection
            .execute(
                "INSERT INTO blobs (data) VALUES (randomblob(?1 * 1048576))",
                [mib],
            )
            .map(drop)
    }
}

/// The items committed to the run file of `env` and its integrity check,
/// read through `vfs` once its writer stopped.
fn committed_through(env: &TestEnv, vfs: &CrashVfs) -> (Vec<i64>, String) {
    let reader = open_reader(&env.folder().join("run.sqlite"), Some(vfs.name())).unwrap();
    let integrity = reader
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    (items(&reader).unwrap(), integrity)
}

/// A full disk met at a statement in the middle of a group, where a write
/// spills past the writer's page cache, ends the group's transaction in
/// SQLite: no write of the group commits, each is answered with the
/// storage failure, and the writer reports it to whoever watches its storage
/// failures, which stop its Run (ADR 0122).
///
/// Covers: row:RT-03
#[test]
fn a_full_disk_at_a_statement_fails_its_whole_group_as_a_storage_failure() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("statement-full"), env.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        ..RunStateOptions::default()
    };

    let (answers, reported) = with_writer_options(&env, &options, async |state| {
        state.write(insert(0)).await.unwrap();
        // A WAL past SQLite's 1,000-page automatic checkpoint, which the
        // writer never runs (ADR 0138).
        state.write(blob(5)).await.unwrap();
        vfs.fail_at(vfs.operations(), IoFault::Full);
        let first = state.reserve().await.unwrap().write(blob(8));
        let second = state.reserve().await.unwrap().write(insert(2));
        let answers = tokio::join!(first, second);
        (answers, state.storage_failures().borrow().is_some())
    });
    vfs.free_space();

    assert_eq!(
        (
            matches!(answers.0, Err(RunStateError::Storage { .. })),
            matches!(answers.1, Err(RunStateError::Storage { .. })),
            reported,
            committed_through(&env, &vfs),
        ),
        (true, true, true, (vec![0], "ok".to_owned())),
        "{answers:?}"
    );
}

/// SQLite reports a read that met an I/O error (EIO) as `SQLITE_CORRUPT`
/// (its Unix VFS's `SQLITE_IOERR_CORRUPTFS`): a write that meets it fails
/// its group as a storage failure, as a full disk does.
///
/// Covers: row:RT-03
#[test]
fn a_write_whose_read_meets_a_damaged_disk_fails_its_group_as_a_storage_failure() {
    let env = TestEnv::new();

    let (answers, reported, committed) = with_writer(&env, 1, async |state| {
        let damaged = state.reserve().await.unwrap().write(|connection| {
            insert(1)(connection)?;
            Err::<(), _>(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                None,
            ))
        });
        let other = state.reserve().await.unwrap().write(insert(2));
        let answers = tokio::join!(damaged, other);
        let reported = state.storage_failures().borrow().is_some();
        (answers, reported, state.read(items).await.unwrap())
    });

    assert_eq!(
        (
            matches!(answers.0, Err(RunStateError::Storage { .. })),
            matches!(answers.1, Err(RunStateError::Storage { .. })),
            reported,
            committed,
        ),
        (true, true, true, Vec::<i64>::new()),
        "{answers:?}"
    );
}

/// A last write stops the writer once it commits, though other handles
/// still hold it: a write through one of them afterwards, or through a
/// place one reserved before, is refused as `Closed`, and the file holds
/// every write queued up to the last and nothing after it (#1763).
/// Covers: story:3-153
#[test]
fn a_last_write_stops_the_writer_while_other_handles_hold_it_and_refuses_later_writes() {
    let env = TestEnv::new();
    let path = env.folder().join("run.sqlite");

    let (stopped, later, reserved) = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            path.clone(),
            &RunStateOptions::default(),
            vec![ITEMS],
            1,
            &owner,
        )
        .await
        .unwrap();
        let stray = state.clone();
        let place = stray.reserve().await.unwrap();
        state.write(insert(1)).await.unwrap();
        state.write_last(insert(2)).await.unwrap();
        let stopped = tokio::time::timeout(Duration::from_mins(1), owner.wait())
            .await
            .is_ok();
        let later = stray.write(insert(3)).await;
        let reserved = place.write(insert(4)).await;
        (stopped, later, reserved)
    });

    assert_eq!(
        (
            stopped,
            later,
            reserved,
            items(&open_reader(&path, None).unwrap()).unwrap(),
        ),
        (
            true,
            Err(RunStateError::Closed),
            Err(RunStateError::Closed),
            vec![1, 2],
        )
    );
}

/// A writer whose run epoch the file no longer holds commits nothing
/// (ADR 0003).
#[test]
fn a_writer_of_an_older_run_epoch_commits_nothing() {
    let env = TestEnv::new();
    with_writer(&env, 2, async |state| {
        state
            .write(|connection| {
                connection
                    .execute(
                        "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at)
                         VALUES ('r', 'collie', 'collie', 'os_user', 2, '{}', 'running', 't')",
                        [],
                    )
                    .map(drop)
            })
            .await
            .unwrap();
    });

    let refused = with_writer(&env, 1, async |state| state.write(insert(1)).await);

    assert_eq!(refused, Err(RunStateError::Fenced { held: 1, found: 2 }));
}

/// Each generated refusal case of run state (ADR 0066): a run file whose
/// header holds the fabricated version is refused, never misread, under
/// ADR 0030's default rule, each refusal's Error code and message
/// snapshotted.
///
/// Covers: story:3-95, format:run-state, error:format_version_unsupported
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[rstest::rstest]
fn each_generated_refusal_of_a_run_file_is_refused(
    #[files("../goodfetch-protocol/generated/refusals/run-state/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: goodfetch_protocol::refusals::RefusalCase = serde_json::from_str(case).unwrap();
    let env = TestEnv::new();
    let found = case.found.to_string();
    with_writer(&env, 1, async |state| {
        state
            .write(move |connection| {
                connection
                    .execute("UPDATE format SET version = ?1", [found])
                    .map(drop)
            })
            .await
            .unwrap();
    });

    let reopened = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let opened = RunState::open(
            env.folder().join("run.sqlite"),
            &RunStateOptions::default(),
            vec![ITEMS],
            1,
            &owner,
        )
        .await
        .map(drop);
        owner.wait().await;
        opened
    });

    let Err(RunStateError::Format {
        format,
        found,
        compatibility,
        install: None,
    }) = reopened
    else {
        panic!("{case:#?} was not refused: {reopened:?}");
    };
    assert_eq!(compatibility, case.compatibility, "{case:#?}");
    let refusal = crate::run::format_refusal(
        &goodfetch_protocol::formats::RUN_STATE,
        &found,
        compatibility,
    );
    insta::assert_snapshot!(
        format!("{format}_{found}"),
        format!("{}: {}", refusal.code(), refusal.message())
    );
}

/// What a writer acknowledged is there when the file opens again.
#[test]
fn acknowledged_writes_are_there_after_the_file_opens_again() {
    let env = TestEnv::new();
    with_writer(&env, 1, async |state| {
        state.write(insert(5)).await.unwrap();
        state.write(insert(6)).await.unwrap();
    });

    let reopened = with_writer(&env, 1, async |state| state.read(items).await.unwrap());

    assert_eq!(reopened, vec![5, 6]);
}

/// What a run file's transactions and its writer's reads count at `full`,
/// beyond its opening, once `test` ran on its writer.
fn counted(test: impl AsyncFnOnce(&RunState)) -> (u64, u64) {
    let env = TestEnv::new();
    let options = RunStateOptions {
        durability: Durability::FULL,
        ..RunStateOptions::default()
    };
    let work = options.work.clone();
    let opening = with_writer_options(&env, &options, async |state| {
        let opening = work.read(Row::RunTransactions);
        test(state).await;
        opening
    });
    (
        work.read(Row::RunTransactions) - opening,
        work.read(Row::WriterReads),
    )
}

/// ADR 0138's note: a transaction is one job on the writer, whatever group it
/// commits in, so three jobs count three whether they commit in one group or
/// in three.
#[test]
fn a_writers_transactions_count_its_jobs_whatever_group_they_commit_in() {
    let apart = counted(async |state| {
        for _ in 0..3 {
            state.write(|_| Ok(())).await.unwrap();
        }
    });
    let grouped = counted(async |state| {
        let (first, second, third) = tokio::join!(
            state.write(|_| Ok(())),
            state.write(|_| Ok(())),
            state.write(|_| Ok(()))
        );
        first.unwrap();
        second.unwrap();
        third.unwrap();
    });

    assert_eq!((apart.0, grouped.0), (3, 3));
}

/// A read leaves the writer (ADR 0138): it runs no job of the writer's, so
/// it counts no transaction and no read through the writer.
#[test]
fn a_read_counts_no_transaction_and_no_read_through_the_writer() {
    let counts = counted(async |state| {
        state.read(items).await.unwrap();
    });

    assert_eq!(counts, (0, 0));
}

/// A read runs beside the writer: one made while a commit is under way
/// answers before that commit ends, with what was committed before it.
#[test]
fn a_read_during_a_commit_does_not_wait_for_it() {
    let env = TestEnv::new();
    let path = env.folder().join("run.sqlite");

    let read = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Threads);
        let state = RunState::open(path, &RunStateOptions::default(), vec![ITEMS], 1, &owner)
            .await
            .unwrap();
        state.write(insert(1)).await.unwrap();
        let (started, starts) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let committing = state.write(move |connection| {
            insert(2)(connection)?;
            let _ = started.send(());
            let _ = released.recv();
            Ok(())
        });
        let reading = async {
            starts.await.unwrap();
            let read = tokio::time::timeout(Duration::from_secs(30), state.read(items)).await;
            release.send(()).unwrap();
            read
        };
        let (committed, read) = tokio::join!(committing, reading);
        committed.unwrap();
        drop(state);
        owner.wait().await;
        read.map(Result::unwrap)
    });

    assert_eq!(read, Ok(vec![1]));
}

/// A run file's reads keep their connections: a read after another runs on
/// the connection that one left, which a setting of its own shows.
#[test]
fn reads_keep_their_connection_from_one_read_to_the_next() {
    let env = TestEnv::new();

    let kept = with_writer(&env, 1, async |state| {
        state
            .read(|connection| connection.pragma_update(None, "cache_size", -7))
            .await
            .unwrap();
        state.read(pragma("cache_size")).await.unwrap()
    });

    assert_eq!(kept, "-7");
}

/// A writer that stops closes its readers first and leaves its WAL beside
/// the file (#2117): a new connection reads every write of a Run that
/// ended, whatever was read beside its writer.
#[test]
fn a_stopped_writers_file_reads_whole_with_its_wal_beside_it() {
    let env = TestEnv::new();

    with_writer(&env, 1, async |state| {
        state.write(insert(1)).await.unwrap();
        state.read(items).await.unwrap();
    });

    let path = env.folder().join("run.sqlite");
    let whole: Vec<i64> = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|connection| items(&connection))
    .unwrap();
    assert_eq!(whole, vec![1]);
}

/// A read commits nothing, so the writer's acknowledged commits do not move
/// for it, and it sees every write acknowledged before it.
#[test]
fn a_read_moves_no_acknowledged_commit() {
    let env = TestEnv::new();

    let (before, seen, after) = with_writer(&env, 1, async |state| {
        state.write(insert(1)).await.unwrap();
        let before = *state.commits().borrow();
        let seen = state.read(items).await.unwrap();
        (before, seen, *state.commits().borrow())
    });

    assert_eq!((seen, after), (vec![1], before));
}

/// A preparation writes nothing, so the writer's acknowledged commits do
/// not move for it; a statement that does not prepare is its answer, and
/// the writer goes on writing.
#[test]
fn a_preparation_commits_nothing_and_answers_a_statement_that_does_not_prepare() {
    let env = TestEnv::new();

    let (before, prepared, after, refused, written) = with_writer(&env, 1, async |state| {
        state.write(insert(1)).await.unwrap();
        let before = *state.commits().borrow();
        let prepared = state
            .prepare(vec!["SELECT value FROM items WHERE value = ?1"])
            .await;
        let after = *state.commits().borrow();
        let refused = state.prepare(vec!["SELECT nothing FROM nowhere"]).await;
        state.write(insert(2)).await.unwrap();
        (before, prepared, after, refused, state.read(items).await)
    });

    assert_eq!((prepared, after), (Ok(()), before));
    assert!(
        matches!(refused, Err(RunStateError::Sqlite { .. })),
        "{refused:?}"
    );
    assert_eq!(written.unwrap(), vec![1, 2]);
}

/// A group that does not commit counts no transaction.
#[test]
fn a_group_the_file_refuses_counts_no_transaction() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register("counted-refused-group", 7).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        ..RunStateOptions::default()
    };
    let work = options.work.clone();

    let (before, refused) = with_writer_options(&env, &options, async |state| {
        let before = work.read(Row::RunTransactions);
        vfs.crash(CrashKind::Process);
        let (first, second) = tokio::join!(state.write(|_| Ok(())), state.write(|_| Ok(())));
        (before, first.is_err() && second.is_err())
    });

    assert!(refused);
    assert_eq!(work.read(Row::RunTransactions), before);
}

/// The Run row a write counts its declared deferred charges on.
fn run_row(connection: &Connection) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at)
             VALUES ('r', 'crawl', 'foxhound', 'os_user', 1, '{}', 'running', 't')",
            [],
        )
        .map(drop)
}

/// What the run file declares of how the Run's charges reach it.
fn declared(connection: &Connection) -> rusqlite::Result<Charges> {
    connection.query_row("SELECT charges FROM run", [], |row| {
        Ok(Charges::stored(&row.get::<_, String>(0)?))
    })
}

/// Runs `test` on a writer at `durability` whose file holds a Run row,
/// opened through a VFS that counts its syncs (ADR 0140), with that VFS.
fn with_counted_writer<T>(
    durability: Durability,
    test: impl AsyncFnOnce(&RunState, &goodfetch_test_support::CountingVfs) -> T,
) -> T {
    let env = TestEnv::new();
    let vfs = goodfetch_test_support::CountingVfs::register();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        durability,
        ..RunStateOptions::default()
    };
    with_writer_options(&env, &options, async |state| {
        state.write(run_row).await.unwrap();
        test(state, &vfs).await
    })
}

/// A write of `value` that charges nothing, deferred.
async fn deferred(state: &RunState, value: i64) {
    state
        .write_with(Syncing::Deferred { charged: 0 }, insert(value))
        .await
        .unwrap();
}

/// ADR 0138's note of #1926: at `full`, a group of deferred writes commits
/// without a sync, and the next group with a write that needs one syncs
/// once, which covers them.
///
/// Covers: story:3-25, setting:run_state.durability
#[test]
fn at_full_deferred_writes_commit_unsynced_and_the_next_synced_group_syncs_once() {
    let added = with_counted_writer(Durability::FULL, async |state, vfs| {
        let mut added = Vec::new();
        for value in [1, 2] {
            let before = vfs.syncs();
            deferred(state, value).await;
            added.push(vfs.syncs() - before);
        }
        let before = vfs.syncs();
        state.write(insert(3)).await.unwrap();
        added.push(vfs.syncs() - before);
        added
    });

    assert_eq!(added, vec![0, 0, 1]);
}

/// One group, whatever it mixes, syncs once at `full` when any of its
/// writes needs a sync, and never at `fast`.
///
/// Covers: story:3-25, story:3-27, setting:run_state.durability
#[rstest::rstest]
#[case::full(Durability::FULL, 1)]
#[case::fast(Durability::FAST, 0)]
fn a_group_with_one_synced_write_syncs_once_at_full(
    #[case] durability: Durability,
    #[case] syncs: u64,
) {
    let added = with_counted_writer(durability, async |state, vfs| {
        let before = vfs.syncs();
        let (first, second, third) = tokio::join!(
            state.write_with(Syncing::Deferred { charged: 0 }, insert(1)),
            state.write(insert(2)),
            state.write_with(Syncing::Deferred { charged: 0 }, insert(3)),
        );
        first.unwrap();
        second.unwrap();
        third.unwrap();
        vfs.syncs() - before
    });

    assert_eq!(added, syncs);
}

/// An Effect's sent mark is synced before it is answered at every
/// Durability level but `off`, where nothing is (ADR 0106, ADR 0138's note
/// of #1550), and a level that syncs no other write goes back to not
/// syncing after it.
///
/// Covers: story:3-34, setting:run_state.durability
#[rstest::rstest]
#[case::full(Durability::FULL, [1, 1])]
#[case::batch(Durability::BATCH, [1, 1])]
#[case::fast(Durability::FAST, [1, 0])]
#[case::periodic(Durability::periodic(Duration::from_secs(3_600)), [1, 0])]
#[case::off(Durability::OFF, [0, 0])]
fn an_effects_sent_mark_syncs_at_every_level_but_off(
    #[case] durability: Durability,
    #[case] syncs: [u64; 2],
) {
    let added = with_counted_writer(durability, async |state, vfs| {
        let before = vfs.syncs();
        state.write_with(Syncing::Sent, insert(1)).await.unwrap();
        let sent = vfs.syncs() - before;
        let before = vfs.syncs();
        state.write(insert(2)).await.unwrap();
        [sent, vfs.syncs() - before]
    });

    assert_eq!(added, syncs);
}

/// A deferred write that charges a Limit defers only once a synced commit
/// declared how much a power cut may take (ADR 0138's note of #1926): the
/// first one syncs and declares [`DEFERRED_CHARGES`], the next ones commit
/// unsynced until what they charged since the last sync would pass it.
///
/// Covers: story:3-25
#[test]
fn deferred_charges_defer_only_within_the_bound_a_synced_commit_declared() {
    let half = DEFERRED_CHARGES / 2;
    let (added, bound) = with_counted_writer(Durability::FULL, async |state, vfs| {
        let mut added = Vec::new();
        for charged in [1, half, half, 1] {
            let before = vfs.syncs();
            state
                .write_with(Syncing::Deferred { charged }, |_| Ok(()))
                .await
                .unwrap();
            added.push(vfs.syncs() - before);
        }
        (added, state.read(declared).await.unwrap())
    });

    // The first declares; the two halves fill the bound; the last would
    // pass it, so its group syncs.
    assert_eq!((added, bound), (vec![1, 0, 0, 1], Charges::Deferred));
}

/// A declaring write, a crawl's start, syncs and declares the bound in its
/// own commit (#2117), so the first deferred charges commit unsynced at
/// `full`, and a power cut after them leaves the bound declared for the
/// reclaim.
///
/// Covers: story:3-25
#[test]
fn a_declaring_write_declares_the_bound_the_first_deferred_charges_defer_within() {
    let half = DEFERRED_CHARGES / 2;
    let env = TestEnv::new();
    let counting = goodfetch_test_support::CountingVfs::register();
    let options = RunStateOptions {
        vfs: Some(counting.name().to_owned()),
        durability: Durability::FULL,
        ..RunStateOptions::default()
    };
    let added = with_writer_options(&env, &options, async |state| {
        let mut added = Vec::new();
        let before = counting.syncs();
        state.write_with(Syncing::Declaring, run_row).await.unwrap();
        added.push(counting.syncs() - before);
        for charged in [1, half, half] {
            let before = counting.syncs();
            state
                .write_with(Syncing::Deferred { charged }, |_| Ok(()))
                .await
                .unwrap();
            added.push(counting.syncs() - before);
        }
        added
    });
    let crashed = TestEnv::new();
    let vfs = CrashVfs::register(&crashed.unique_name("declared-bound"), crashed.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        durability: Durability::FULL,
        ..RunStateOptions::default()
    };
    with_writer_options(&crashed, &options, async |state| {
        state.write_with(Syncing::Declaring, run_row).await.unwrap();
        state
            .write_with(Syncing::Deferred { charged: 1 }, insert(1))
            .await
            .unwrap();
        vfs.crash(CrashKind::PowerCut);
    });
    let path = crashed.folder().join("run.sqlite");
    let after = goodfetch_test_support::after_crash(&vfs, &path.to_string_lossy()).unwrap();

    // 1 and a half fit the bound; the second half would pass it.
    assert_eq!(
        (added, declared(&after).unwrap()),
        (vec![1, 0, 0, 1], Charges::Deferred)
    );
}

/// What a run file declares of the Run's charges once its Run's row is
/// written, before any write charges (ADR 0142's note of G-11 Q3): nothing
/// at a level whose charges are synced as they are answered, `unsynced`
/// at a `run_state.batch.sync` other than `each`, whatever writes it.
///
/// Covers: story:3-25, setting:run_state.durability, setting:run_state.batch.sync
#[rstest::rstest]
#[case::full(Durability::FULL, Charges::Synced)]
#[case::batch(Durability::BATCH, Charges::Synced)]
#[case::fast(Durability::FAST, Charges::Unsynced)]
#[case::periodic(Durability::periodic(Duration::from_secs(60)), Charges::Unsynced)]
#[case::off(Durability::OFF, Charges::Unsynced)]
fn a_run_file_declares_its_charges_unsynced_from_its_start_at_a_sync_other_than_each(
    #[case] durability: Durability,
    #[case] charges: Charges,
) {
    let declared = with_counted_writer(durability, async |state, _| {
        state.write(insert(1)).await.unwrap();
        state.read(declared).await.unwrap()
    });

    assert_eq!(declared, charges);
}

/// At `batch`, a deferred write that charges before the file declares its
/// charges unsynced declares them, answered once its batch is in the file;
/// the charges after it ride the open batch (ADR 0142's note of G-11 Q3).
///
/// Covers: story:3-25, setting:run_state.durability
#[test]
fn at_batch_the_first_charge_declares_the_charges_unsynced_and_the_next_ride() {
    let found = with_counted_writer(Durability::BATCH, async |state, _| {
        let mut found = Vec::new();
        for value in [1, 2] {
            state
                .write_with(Syncing::Deferred { charged: 1 }, insert(value))
                .await
                .unwrap();
            found.push((
                state.read(items).await.unwrap(),
                state.read(declared).await.unwrap(),
            ));
        }
        found
    });

    assert_eq!(
        found,
        vec![(vec![1], Charges::Unsynced), (vec![1], Charges::Unsynced)]
    );
}

/// A run file's declaration only rises: a Run that declared its charges
/// unsynced at `batch` and resumes at `full` keeps them declared so, and
/// its charges defer within [`DEFERRED_CHARGES`] without declaring again.
///
/// Covers: story:3-25
#[test]
fn a_declaration_only_rises_when_the_run_resumes_at_another_level() {
    let env = TestEnv::new();
    let at = |durability| RunStateOptions {
        durability,
        ..RunStateOptions::default()
    };
    with_writer_options(&env, &at(Durability::BATCH), async |state| {
        state.write_with(Syncing::Declaring, run_row).await.unwrap();
    });

    let resumed = with_writer_options(&env, &at(Durability::FULL), async |state| {
        state
            .write_with(Syncing::Declaring, |_| Ok(()))
            .await
            .unwrap();
        state
            .write_with(Syncing::Deferred { charged: 1 }, |_| Ok(()))
            .await
            .unwrap();
        state.read(declared).await.unwrap()
    });

    assert_eq!(resumed, Charges::Unsynced);
}

/// What a run file at `durability` syncs through a counting VFS (ADR 0140)
/// as its writer opens it new, as a write that needs a sync commits, as its
/// last write, a clean stop, commits, and once the writer stopped.
fn syncs_of_a_run(durability: Durability) -> [u64; 4] {
    let env = TestEnv::new();
    let vfs = goodfetch_test_support::CountingVfs::register();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        durability,
        ..RunStateOptions::default()
    };
    let path = env.folder().join("run.sqlite");
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(path, &options, vec![ITEMS], 1, &owner)
            .await
            .unwrap();
        let opened = vfs.syncs();
        state.write(insert(1)).await.unwrap();
        let written = vfs.syncs();
        state.write_last(insert(2)).await.unwrap();
        let last = vfs.syncs();
        drop(state);
        owner.wait().await;
        [opened, written - opened, last - written, vfs.syncs() - last]
    })
}

/// A run file's own syncs fold into the commits it acknowledges (#2117): a
/// new file's switch to WAL syncs its rollback journal once and the file
/// once; its first commit, its tables, and the WAL's header go unsynced
/// below `off`, where its "unclean" mark syncs; the next synced commit
/// covers them. The clean stop syncs its last write at every level and
/// leaves the WAL, which that sync covers, as it is: no checkpoint syncs
/// the WAL and the file again.
///
/// Covers: story:3-25, story:3-27, setting:run_state.durability
#[rstest::rstest]
#[case::batch(Durability::BATCH, [2, 1, 1, 0])]
#[case::full(Durability::FULL, [2, 1, 1, 0])]
#[case::fast(Durability::FAST, [2, 0, 1, 0])]
#[case::periodic(Durability::periodic(Duration::from_secs(3600)), [2, 0, 1, 0])]
#[case::off(Durability::OFF, [4, 0, 1, 0])]
fn a_run_file_syncs_its_switch_to_wal_what_it_acknowledges_and_its_clean_stop(
    #[case] durability: Durability,
    #[case] syncs: [u64; 4],
) {
    assert_eq!(syncs_of_a_run(durability), syncs);
}

/// The guarantees the run file's folded syncs keep (#2117, ADR 0138's
/// Durability levels): through a power cut, a new file's first synced
/// write is there at `full`, with the tables its opening committed
/// unsynced, and a clean stop's writes are there at every level, the file
/// whole and its "unclean" mark cleared, though no checkpoint copied the
/// WAL into it.
///
/// Covers: story:3-25, story:3-27, setting:run_state.durability
#[rstest::rstest]
#[case::full_synced_write(Durability::FULL, false, vec![1])]
#[case::full_clean_stop(Durability::FULL, true, vec![1, 2])]
#[case::batch_clean_stop(Durability::BATCH, true, vec![1, 2])]
#[case::fast_clean_stop(Durability::FAST, true, vec![1, 2])]
#[case::periodic_clean_stop(Durability::periodic(Duration::from_secs(3600)), true, vec![1, 2])]
#[case::off_clean_stop(Durability::OFF, true, vec![1, 2])]
fn what_a_run_file_acknowledged_survives_a_power_cut_after_its_folded_syncs(
    #[case] durability: Durability,
    #[case] stops: bool,
    #[case] survived: Vec<i64>,
) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("folded-syncs"), env.seed()).unwrap();
    let options = RunStateOptions {
        vfs: Some(vfs.name().to_owned()),
        durability,
        ..RunStateOptions::default()
    };
    let path = env.folder().join("run.sqlite");

    with_writer_options(&env, &options, async |state| {
        if stops {
            state
                .write_with(Syncing::Deferred { charged: 0 }, insert(1))
                .await
                .unwrap();
            state.write_last(insert(2)).await.unwrap();
        } else {
            state.write(insert(1)).await.unwrap();
            vfs.crash(CrashKind::PowerCut);
        }
    });
    if stops {
        vfs.crash(CrashKind::PowerCut);
    }
    let after = goodfetch_test_support::after_crash(&vfs, &path.to_string_lossy()).unwrap();

    assert_eq!(
        (
            items(&after).unwrap(),
            unclean(&path, Some(vfs.name()), durability).unwrap()
        ),
        (survived, false)
    );
}
