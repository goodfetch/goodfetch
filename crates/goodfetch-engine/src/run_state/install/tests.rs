//! The install database through its interface (spec #213's Seam 4): run
//! files opened through its VFS live inside the one database, each apart
//! from the others, readable beside their writer, and the same run file
//! comes out of it as out of a file of its own. Its crashes are in
//! `run_state`'s model-based test.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests list the run folders on disk themselves"
)]

use std::path::{Path, PathBuf};

use goodfetch_test_support::TestEnv;
use rusqlite::{Connection, OpenFlags};

use super::{INSTALL_FILE, InstallDatabase};
use crate::run_state::tests::{ITEMS, insert, items};
use crate::run_state::{RunState, RunStateOptions, open_reader};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// An install database in the test's folder, whose run folders are under
/// `runs`.
struct Install {
    database: InstallDatabase,
    runs: PathBuf,
    path: PathBuf,
}

fn install(env: &TestEnv) -> Install {
    let runs = env.folder().join("runs");
    let path = env.folder().join(INSTALL_FILE);
    let database = InstallDatabase::open(&path, &runs, &RunStateOptions::default()).unwrap();
    Install {
        database,
        runs,
        path,
    }
}

impl Install {
    fn options(&self) -> RunStateOptions {
        RunStateOptions {
            vfs: Some(self.database.vfs().to_owned()),
            ..RunStateOptions::default()
        }
    }

    fn run_file(&self, run_id: &str) -> PathBuf {
        self.runs.join(run_id).join("run.sqlite")
    }

    /// Writes `values` through the writer of `run_id`'s run file, then
    /// stops it.
    fn write(&self, run_id: &str, values: &[i64]) {
        let (path, options, values) = (self.run_file(run_id), self.options(), values.to_vec());
        goodfetch_test_support::runtime().block_on(async move {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let state = RunState::open(path, &options, vec![ITEMS], 1, &owner)
                .await
                .unwrap();
            for value in values {
                state.write(insert(value)).await.unwrap();
            }
            drop(state);
            owner.wait().await;
        });
    }

    fn read(&self, run_id: &str) -> Vec<i64> {
        let reader = open_reader(&self.run_file(run_id), Some(self.database.vfs())).unwrap();
        items(&reader).unwrap()
    }
}

/// What the files under `folder` are, by their path below it.
fn files_under(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path.to_string_lossy().into_owned());
            }
        }
    }
    found
}

/// Covers: story:3-29, setting:run_state.layout
#[test]
fn run_files_of_the_per_install_layout_live_in_the_one_database_each_apart() {
    let env = TestEnv::new();
    let install = install(&env);

    install.write("first", &[1, 2]);
    install.write("second", &[3]);

    assert_eq!(
        (
            install.read("first"),
            install.read("second"),
            files_under(&install.runs),
            install.path.is_file(),
        ),
        (vec![1, 2], vec![3], Vec::<String>::new(), true)
    );
}

/// A run file is kept across closing and opening the install database
/// again, as across a Daemon's restart.
#[test]
fn a_run_file_is_there_after_the_install_database_opens_again() {
    let env = TestEnv::new();
    let first = install(&env);
    first.write("kept", &[7]);
    drop(first);

    let again = install(&env);

    assert_eq!(again.read("kept"), vec![7]);
}

/// A reader beside the writer sees each acknowledged commit, never one in
/// the making.
#[test]
fn a_reader_beside_the_writer_sees_what_it_acknowledged() {
    let env = TestEnv::new();
    let install = install(&env);
    let (path, options) = (install.run_file("beside"), install.options());

    let seen = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(path, &options, vec![ITEMS], 1, &owner)
            .await
            .unwrap();
        state.write(insert(1)).await.unwrap();
        let before = install.read("beside");
        state.write(insert(2)).await.unwrap();
        let after = install.read("beside");
        drop(state);
        owner.wait().await;
        (before, after)
    });

    assert_eq!(seen, (vec![1], vec![1, 2]));
}

/// The run file's bytes, copied out through SQLite's backup, open as a
/// file of its own with the same rows: the per-run file both layouts give.
#[test]
fn a_run_file_copied_out_of_the_install_database_is_a_file_of_its_own() {
    let env = TestEnv::new();
    let install = install(&env);
    install.write("out", &[4, 5]);
    let copy = env.folder().join("out.sqlite");

    {
        let reader = open_reader(&install.run_file("out"), Some(install.database.vfs())).unwrap();
        let mut into = Connection::open(&copy).unwrap();
        let backup = rusqlite::backup::Backup::new(&reader, &mut into).unwrap();
        backup.step(-1).unwrap();
    }
    let opened = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

    assert_eq!(items(&opened).unwrap(), vec![4, 5]);
}

/// A second install database on the same run folders in one process is
/// refused, so one writer keeps them.
#[test]
fn a_second_install_database_on_the_same_run_folders_is_refused() {
    let env = TestEnv::new();
    let _first = install(&env);

    let second = InstallDatabase::open(
        &env.folder().join(INSTALL_FILE),
        &env.folder().join("runs"),
        &RunStateOptions::default(),
    );

    assert!(second.is_err());
}

/// A path outside the install database's run folders does not open
/// through its VFS.
#[test]
fn a_path_outside_the_run_folders_does_not_open_through_its_vfs() {
    let env = TestEnv::new();
    let install = install(&env);

    let outside = Connection::open_with_flags_and_vfs(
        env.folder().join("elsewhere.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        install.database.vfs(),
    );

    assert!(outside.is_err());
}

/// Erasing a Run's files deletes those of its run folder only, a Run ID
/// that begins with another's included, and finds them by a range on the
/// primary key, so the erase holds the database for what the erased Run
/// keeps, not for every Run's files (deep review 213-3).
#[test]
fn erasing_a_runs_files_searches_their_names_and_leaves_every_other_run() {
    let env = TestEnv::new();
    let install = install(&env);
    for run_id in ["a", "a_b", "b"] {
        install.write(run_id, &[1]);
    }

    let erased = install.database.erase("a").unwrap();

    let connection = Connection::open_with_flags(
        &install.path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let plans: Vec<String> = ["chunks", "files"]
        .into_iter()
        .map(|table| {
            connection
                .query_row(
                    &format!(
                        "EXPLAIN QUERY PLAN DELETE FROM {table} WHERE {}",
                        super::ERASED
                    ),
                    ["a/", "a0"],
                    |row| row.get::<_, String>(3),
                )
                .unwrap()
        })
        .collect();
    assert_eq!(
        (
            erased,
            install.database.runs().unwrap(),
            plans.iter().all(|plan| plan.starts_with("SEARCH")),
        ),
        (true, vec!["a_b".to_owned(), "b".to_owned()], true),
        "{plans:?}"
    );
}
