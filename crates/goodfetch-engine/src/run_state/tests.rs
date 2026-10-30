//! `run_state` through its interface, on real SQLite in the test's folder or
//! through the crash VFS (spec #213's Seam 4): durability pragmas, grouped
//! commits under the run epoch, format refusals, and a model-based test of
//! crashes, power cuts and reopening against a reference model (ADR 0060).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod model;

use std::time::Duration;

use goodfetch_test_support::{CrashVfs, IoFault, TestEnv};
use rusqlite::Connection;

use super::{
    Durability, PAGE_CACHE_KIB, RunState, RunStateError, RunStateOptions, RunStateTables,
    open_reader,
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
            state.read(pragma("synchronous")).await.unwrap(),
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
        durability: Durability::Fast,
        ..RunStateOptions::default()
    };

    let pragmas = with_writer_options(&env, &options, async |state| {
        (
            state.read(pragma("journal_mode")).await.unwrap(),
            state.read(pragma("synchronous")).await.unwrap(),
        )
    });

    assert_eq!(pragmas, ("wal".to_owned(), "1".to_owned()));
}

/// The writer's page cache is bounded at `PAGE_CACHE_KIB`, so a Frontier
/// of any size keeps its memory under ADR 0134's 8 bytes per URL at
/// 100,000 URLs, its visited set staying on disk.
#[test]
fn a_run_file_bounds_its_page_cache() {
    let env = TestEnv::new();

    let cache_size = with_writer(&env, 1, async |state| {
        state.read(pragma("cache_size")).await.unwrap()
    });

    assert_eq!(cache_size, format!("-{PAGE_CACHE_KIB}"));
}

/// Covers: story:3-26
#[cfg(target_os = "macos")]
#[test]
fn a_run_file_syncs_with_fullfsync_on_macos() {
    let env = TestEnv::new();

    let fullfsync = with_writer(&env, 1, async |state| {
        (
            state.read(pragma("fullfsync")).await.unwrap(),
            state.read(pragma("checkpoint_fullfsync")).await.unwrap(),
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
/// SQLite: no write of the group commits, though a small one still fits in
/// the room an earlier checkpoint left in the WAL, each is answered with the
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
        // Past SQLite's 1,000-page automatic checkpoint, whose WAL the next
        // commit writes again from its start.
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
