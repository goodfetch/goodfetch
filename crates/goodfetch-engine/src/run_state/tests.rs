//! `run_state` through its interface, on real SQLite in the test's folder or
//! through the crash VFS (spec #213's Seam 4): durability pragmas, grouped
//! commits under the run epoch, format refusals, and a model-based test of
//! crashes, power cuts and reopening against a reference model (ADR 0060).

mod model;

use goodfetch_test_support::TestEnv;
use rusqlite::Connection;

use super::{RunState, RunStateError, RunStateOptions, RunStateTables};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A contributed table the tests write through the writer.
const ITEMS: RunStateTables = RunStateTables {
    name: "test",
    statements: &["CREATE TABLE IF NOT EXISTS items (value INTEGER NOT NULL) STRICT"],
};

fn insert(value: i64) -> impl FnOnce(&Connection) -> rusqlite::Result<()> + Send + 'static {
    move |connection| {
        connection
            .execute("INSERT INTO items (value) VALUES (?1)", [value])
            .map(drop)
    }
}

fn items(connection: &Connection) -> rusqlite::Result<Vec<i64>> {
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
    let path = env.folder().join("run.sqlite");
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            path,
            &RunStateOptions::default(),
            vec![ITEMS],
            epoch,
            &owner,
        )
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
