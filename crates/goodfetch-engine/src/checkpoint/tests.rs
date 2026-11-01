//! `checkpoint` through its interface, on real SQLite in the test's folder
//! (spec #213's Seam 4): a Checkpoint holds the run file as of its last
//! commit, with the run epoch and journal position it reports and the size
//! and digest of the bytes that landed; it is a self-contained file; a
//! write that fails leaves no file; reading one refuses a format version
//! this engine cannot read; installing one makes it a run file. Fuzz
//! replay of the Checkpoint target's seeds and crashers (ADR 0067).

mod model;

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::RunBinding;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

use super::{Held, Written, install, open_checkpoint, read, write};
use crate::run_state::{RunState, RunStateError, RunStateOptions, RunStateTables};
use crate::tasks::{BlockingPolicy, TaskOwner};

const ITEMS: RunStateTables = RunStateTables {
    name: "test",
    statements: &["CREATE TABLE IF NOT EXISTS items (value INTEGER NOT NULL) STRICT"],
};

/// The `run` row of a Run `kept` under `epoch`, bound as `binding` says.
fn insert_run(
    epoch: u64,
    binding: Option<&'static str>,
) -> impl FnOnce(&Connection) -> rusqlite::Result<()> + Send + 'static {
    move |connection| {
        connection
            .execute(
                "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at, binding)
                 VALUES ('kept', 'collie', 'collie', 'os_user', ?1, '{}', 'running', '2026-01-05T09:00:00Z', ?2)",
                (i64::try_from(epoch).unwrap(), binding),
            )
            .map(drop)
    }
}

fn insert(value: i64) -> impl FnOnce(&Connection) -> rusqlite::Result<()> + Send + 'static {
    move |connection| {
        connection
            .execute("INSERT INTO items (value) VALUES (?1)", [value])
            .map(drop)
    }
}

fn items(connection: &Connection) -> Vec<i64> {
    let mut statement = connection
        .prepare("SELECT value FROM items ORDER BY value")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<i64>>>()
        .unwrap()
}

/// Opens a run file in the test's folder under `epoch`, writes the Run's
/// row bound as `binding` says and `values`, one commit each, then runs
/// `then` on its path while the writer is open.
fn with_run<T>(
    env: &TestEnv,
    epoch: u64,
    binding: Option<&'static str>,
    values: &[i64],
    then: impl FnOnce(&Path) -> T,
) -> T {
    let path = env.folder().join("run.sqlite");
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            path.clone(),
            &RunStateOptions::default(),
            vec![ITEMS],
            epoch,
            &owner,
        )
        .await
        .unwrap();
        state.write(insert_run(epoch, binding)).await.unwrap();
        for value in values {
            state.write(insert(*value)).await.unwrap();
        }
        let answer = then(&path);
        drop(state);
        owner.wait().await;
        answer
    })
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

/// Covers: story:3-30
#[test]
fn a_checkpoint_holds_the_committed_run_file_and_reports_its_epoch_position_size_and_digest() {
    let env = TestEnv::new();
    let destination = env.folder().join("elsewhere").join("taken.sqlite");

    let written = with_run(&env, 3, None, &[1, 2], |source| {
        write(source, None, &destination, 7).unwrap()
    });

    let copy = Connection::open(&destination).unwrap();
    let journal_mode: String = copy
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(
        (
            written,
            items(&copy),
            journal_mode,
            std::fs::read_dir(env.folder().join("elsewhere"))
                .unwrap()
                .count(),
        ),
        (
            Written {
                size: std::fs::metadata(&destination).unwrap().len(),
                digest: sha256(&destination),
                run_epoch: 3,
                journal_position: 3,
            },
            vec![1, 2],
            "delete".to_owned(),
            1,
        )
    );
}

/// A destination its folder refuses: the write fails as the OS says, and
/// leaves no file, temporary or not.
/// Covers: story:3-30
#[cfg(unix)]
#[test]
fn a_checkpoint_that_cannot_be_written_leaves_no_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let env = TestEnv::new();
    let refusing = env.folder().join("refusing");
    std::fs::create_dir(&refusing).unwrap();
    std::fs::set_permissions(&refusing, std::fs::Permissions::from_mode(0o555)).unwrap();

    let failed = with_run(&env, 1, None, &[1], |source| {
        write(source, None, &refusing.join("taken.sqlite"), 7)
    });

    std::fs::set_permissions(&refusing, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Err(super::WriteError::Storage {
        reason, os_code, ..
    }) = failed
    else {
        panic!("the write did not fail on its storage: {failed:?}");
    };
    assert_eq!(
        (
            reason,
            os_code,
            std::fs::read_dir(&refusing).unwrap().count()
        ),
        ("permission_denied", Some("EACCES"), 0)
    );
}

/// Covers: story:3-34
#[test]
fn reading_a_checkpoint_gives_its_run_its_product_lifecycle_and_binding() {
    let env = TestEnv::new();
    let destination = env.folder().join("taken.sqlite");
    with_run(
        &env,
        2,
        Some(r#"{"kind":"machine","machine":"m-1"}"#),
        &[],
        |source| write(source, None, &destination, 7).unwrap(),
    );

    let held = read(&open_checkpoint(&destination).unwrap(), &destination);

    assert_eq!(
        held.unwrap(),
        Held {
            run_id: "kept".to_owned(),
            needs: Product::Collie,
            lifecycle: Lifecycle::Running,
            binding: Some(RunBinding::Machine {
                machine: "m-1".to_owned()
            }),
            run_epoch: 2,
            key: None,
        }
    );
}

/// Fabricated versions of the run-state format in a real Checkpoint: a
/// newer minor or major is refused, and the file is left as it was.
/// Covers: story:3-38
#[rstest::rstest]
#[case::a_newer_preview("0.2", Compatibility::NewerMajor)]
#[case::a_newer_major("1.0", Compatibility::NewerMajor)]
fn a_checkpoint_of_a_version_this_engine_cannot_read_is_refused(
    #[case] version: &str,
    #[case] compatibility: Compatibility,
) {
    let env = TestEnv::new();
    let destination = env.folder().join("taken.sqlite");
    with_run(&env, 1, None, &[], |source| {
        write(source, None, &destination, 7).unwrap()
    });
    Connection::open(&destination)
        .unwrap()
        .execute(
            "UPDATE format SET version = ?1 WHERE name = 'run-state'",
            [version],
        )
        .unwrap();
    let before = sha256(&destination);

    let refused = read(&open_checkpoint(&destination).unwrap(), &destination);

    assert_eq!(
        (refused.map(|_| ()), sha256(&destination)),
        (
            Err(RunStateError::Format {
                format: "run-state",
                found: version.to_owned(),
                compatibility,
                install: None,
            }),
            before
        )
    );
}

/// An earlier run file of the Run, in WAL mode with a commit of its own,
/// is replaced whole by the Checkpoint installed over it.
/// Covers: story:3-34
#[test]
fn an_installed_checkpoint_replaces_the_run_file_whole() {
    let env = TestEnv::new();
    let checkpoint = env.folder().join("taken.sqlite");
    with_run(&env, 1, None, &[4, 5], |source| {
        write(source, None, &checkpoint, 7).unwrap()
    });
    let machine = env.folder().join("other machine");
    std::fs::create_dir(&machine).unwrap();
    let run_file = machine.join("run.sqlite");
    let earlier = Connection::open(&run_file).unwrap();
    earlier
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE items (value INTEGER NOT NULL) STRICT;
             INSERT INTO items (value) VALUES (9);",
        )
        .unwrap();

    install(&checkpoint, &run_file, None).unwrap();

    drop(earlier);
    let installed = Connection::open(&run_file).unwrap();
    let epoch: i64 = installed
        .query_row("SELECT epoch FROM run", [], |row| row.get(0))
        .unwrap();
    assert_eq!((items(&installed), epoch), (vec![4, 5], 1));
}

/// A Checkpoint whose kept files' chunks are damaged is refused before it
/// touches the run file it would replace: that run file keeps its own
/// state, and none of the Checkpoint's chunks.
/// Covers: story:3-34
#[test]
fn a_checkpoint_whose_kept_files_cannot_be_carried_leaves_the_run_file_as_it_was() {
    let (env, other) = (TestEnv::new(), TestEnv::new());
    let checkpoint = env.folder().join("taken.sqlite");
    with_run(&env, 1, None, &[4, 5], |source| {
        write(source, None, &checkpoint, 7).unwrap()
    });
    let damaged = Connection::open(&checkpoint).unwrap();
    for statement in crate::run_files::TABLES.statements {
        damaged.execute(statement, []).unwrap();
    }
    damaged
        .execute(
            "INSERT INTO run_file_chunks (sha256, offset, data) VALUES (?1, 0, x'00')",
            ["a".repeat(64)],
        )
        .unwrap();
    drop(damaged);
    let run_file = with_run(&other, 1, None, &[9], Path::to_owned);

    let installed = install(&checkpoint, &run_file, None);

    let kept = Connection::open(&run_file).unwrap();
    let chunks: i64 = kept
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'run_file_chunks'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (installed.is_err(), items(&kept), chunks),
        (true, vec![9], 0)
    );
}

mod checkpoint_file {
    goodfetch_test_support::fuzz_replay!(crate::checkpoint::fuzz_checkpoint, "fuzz/checkpoint");
}
