//! The Politeness file through its interface, on real SQLite in the test's
//! folder, with two engines' handles sharing one file (spec #213's Seam 4):
//! the combined count under a key against each run's own cap, waiters woken
//! in turn through their wake endpoints, a woken waiter still refused
//! passing the wake on, the Permits of a gone Run reclaimed, the file's
//! format, and a take whose transaction meets a full disk.
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the wall clock as its rules' reading, and opens the file to look at it"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::formats::Compatibility;
use goodfetch_test_support::{CrashVfs, IoFault, TestEnv};
use rusqlite::{Connection, OpenFlags};
use tokio::sync::broadcast;

use super::{
    Ending, Holder, Permit, PermitRule, Politeness, PolitenessError, PolitenessOptions, Taken,
    Verdict, Wake,
};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A run's own cap on the count in flight under a key, and nothing else.
struct Cap(u64);

impl PermitRule for Cap {
    fn admit(
        &self,
        _connection: &Connection,
        _key: &str,
        in_flight: u64,
        _now: SystemTime,
    ) -> rusqlite::Result<Verdict> {
        Ok(if in_flight < self.0 {
            Verdict::Go
        } else {
            Verdict::Busy
        })
    }

    fn started(&self, _: &Connection, _: &str, _: SystemTime) -> rusqlite::Result<()> {
        Ok(())
    }

    fn ended(
        &self,
        _: &Connection,
        _: &str,
        _: u64,
        _: Ending,
        _: SystemTime,
    ) -> rusqlite::Result<Option<SystemTime>> {
        Ok(None)
    }
}

fn cap(cap: u64) -> Arc<dyn PermitRule> {
    Arc::new(Cap(cap))
}

/// A Run of the test's: live unless its folder's name says it is gone.
fn holder(env: &TestEnv, run_id: &str) -> Holder {
    Holder {
        run_id: run_id.to_owned(),
        epoch: 1,
        run_folder: env.folder().join(run_id),
    }
}

fn gone_unless_live(folder: &Path, _epoch: u64) -> bool {
    folder
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("gone"))
}

async fn engine(env: &TestEnv, name: &str, owner: &TaskOwner) -> Politeness {
    Politeness::open(
        env.home().join("politeness.sqlite"),
        &PolitenessOptions::default(),
        Vec::new(),
        gone_unless_live,
        name.to_owned(),
        owner,
    )
    .await
    .unwrap()
}

fn granted(taken: Taken) -> Permit {
    match taken {
        Taken::Granted(permit) => permit,
        Taken::Wait(until) => panic!("held until {until:?}"),
        Taken::Refused(until) => panic!("refused, unsent, until {until:?}"),
    }
}

/// The wakes `wakes` holds now, without waiting for more.
fn received(wakes: &mut broadcast::Receiver<Wake>) -> Vec<String> {
    std::iter::from_fn(|| wakes.try_recv().ok())
        .map(|wake| format!("{}@{}", wake.run_id, wake.key))
        .collect()
}

/// Waits for the next wake `wakes` receives, at most a few seconds.
async fn next_wake(wakes: &mut broadcast::Receiver<Wake>) -> String {
    let wake = tokio::time::timeout(Duration::from_secs(10), wakes.recv())
        .await
        .expect("a wake arrives")
        .unwrap();
    format!("{}@{}", wake.run_id, wake.key)
}

fn run<T>(test: impl AsyncFnOnce(&TaskOwner) -> T) -> T {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let answer = test(&owner).await;
        owner.cancel();
        answer
    })
}

/// An engine whose first Run opens the user's new Politeness file while
/// another engine writes it opens the file once that write ends and takes
/// its Permit (#1774): its switch of the file to WAL waits, as SQLite
/// answers that switch busy at once, where a Run starting beside another
/// on a new home failed `database is locked`.
///
/// Covers: story:3-80
#[test]
fn an_engine_opening_the_new_file_beside_another_engines_write_takes_its_permit() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let taken = crate::sqlite_wal::tests::opened_beside_a_write(
        &env.home().join("politeness.sqlite"),
        || {
            run(async |owner| {
                let politeness = Politeness::open(
                    env.home().join("politeness.sqlite"),
                    &PolitenessOptions::default(),
                    Vec::new(),
                    gone_unless_live,
                    "second".to_owned(),
                    owner,
                )
                .await?;
                politeness
                    .take("shop.test", &holder(&env, "second"), cap(1), now)
                    .await
                    .map(|taken| matches!(taken, Taken::Granted(_)))
            })
        },
    );

    assert_eq!(taken, Ok(true));
}

/// Covers: story:3-80, story:3-81
#[test]
fn a_run_admits_only_while_the_combined_count_under_its_key_is_below_its_own_cap() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let answers = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let second = engine(&env, "second", owner).await;
        let _held = granted(
            first
                .take("shop.test", &holder(&env, "loose"), cap(2), now)
                .await
                .unwrap(),
        );
        let strict = second
            .take("shop.test", &holder(&env, "strict"), cap(1), now)
            .await
            .unwrap();
        let looser = second
            .take("shop.test", &holder(&env, "looser"), cap(2), now)
            .await
            .unwrap();
        let elsewhere = second
            .take("other.test", &holder(&env, "strict"), cap(1), now)
            .await
            .unwrap();
        (
            strict,
            matches!(looser, Taken::Granted(_)),
            matches!(elsewhere, Taken::Granted(_)),
        )
    });

    assert_eq!(answers, (Taken::Wait(None), true, true));
}

/// A take whose transaction meets the crash VFS's full disk is rolled back
/// and never acknowledged: it fails with `Storage`, the file's storage
/// failures say so, which stop the Runs taking its Permits, it holds no
/// Permit for it, and `PRAGMA integrity_check` passes (ADR 0122).
///
/// Covers: row:RT-03
#[test]
fn a_take_that_meets_a_full_disk_is_rolled_back_and_reported_as_a_storage_failure() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("politeness-full"), env.seed()).unwrap();
    let file = env.home().join("politeness.sqlite");
    let now = SystemTime::now();

    let (failed, reported) = run(async |owner| {
        let politeness = Politeness::open(
            file.clone(),
            &PolitenessOptions {
                vfs: Some(vfs.name().to_owned()),
                ..PolitenessOptions::default()
            },
            Vec::new(),
            gone_unless_live,
            "engine".to_owned(),
            owner,
        )
        .await
        .unwrap();
        let _held = granted(
            politeness
                .take("shop.test", &holder(&env, "first"), cap(4), now)
                .await
                .unwrap(),
        );
        vfs.fail_at(vfs.operations(), IoFault::Full);
        let failed = politeness
            .take("shop.test", &holder(&env, "second"), cap(4), now)
            .await;
        let reported = politeness.storage_failures().borrow().is_some();
        (failed, reported)
    });

    let connection = Connection::open_with_flags_and_vfs(
        &file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    let holders: Vec<String> = connection
        .prepare("SELECT run_id FROM permits ORDER BY permit_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (
            matches!(failed, Err(PolitenessError::Storage { .. })),
            reported,
            holders,
            integrity,
        ),
        (true, true, vec!["first".to_owned()], "ok".to_owned()),
        "{failed:?}"
    );
}

/// Under `BlockingPolicy::Inline`, the Simulation's and the Compatibility
/// corpus writers', the Politeness file opens with no thread of the
/// runtime's blocking pool, here all held by another job: work on such a
/// thread lets the other tasks run for as long as the thread takes, so a
/// run's interleaving, which writes its group commits and its journal
/// position, hung on the host's load (the crawl run-state head's journal
/// position flipped between two values under #1924).
#[test]
fn a_politeness_file_opened_under_inline_blocking_takes_no_thread_of_the_runtime() {
    let env = TestEnv::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (holding, holds) = std::sync::mpsc::channel::<()>();

    let opened = runtime.block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let pool = tokio::runtime::Handle::current().spawn_blocking(move || {
            holding.send(()).unwrap();
            let _ = held.recv();
        });
        holds.recv().unwrap();
        let opened = tokio::time::timeout(
            Duration::from_secs(5),
            Politeness::open(
                env.home().join("politeness.sqlite"),
                &PolitenessOptions::default(),
                Vec::new(),
                gone_unless_live,
                "inline".to_owned(),
                &owner,
            ),
        )
        .await
        .map(|opened| opened.map(drop).map_err(|error| error.to_string()));
        release.send(()).unwrap();
        pool.await.unwrap();
        owner.cancel();
        opened
    });

    assert_eq!(opened, Ok(Ok(())), "the open waited for the blocking pool");
}

/// Covers: story:3-80
#[test]
fn a_permit_given_back_wakes_the_waiter_whose_turn_it_is_through_its_wake_endpoint() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let (woken, others) = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let second = engine(&env, "second", owner).await;
        let third = engine(&env, "third", owner).await;
        let mut second_wakes = second.wakes();
        let mut third_wakes = third.wakes();
        let held = granted(
            first
                .take("shop.test", &holder(&env, "holding"), cap(1), now)
                .await
                .unwrap(),
        );
        second
            .take("shop.test", &holder(&env, "waiting-first"), cap(1), now)
            .await
            .unwrap();
        third
            .take("shop.test", &holder(&env, "waiting-second"), cap(1), now)
            .await
            .unwrap();

        first
            .give_back(held, cap(1), Ending::Done, now)
            .await
            .unwrap();

        let woken = next_wake(&mut second_wakes).await;
        (woken, received(&mut third_wakes))
    });

    assert_eq!(
        (woken, others),
        ("waiting-first@shop.test".to_owned(), Vec::<String>::new())
    );
}

/// Covers: story:5-90, setting:politeness.sharing
#[test]
fn a_permit_given_back_to_a_runs_own_politeness_wakes_its_waiter_within_the_engine() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let woken = run(async |owner| {
        let own = Politeness::in_memory(&PolitenessOptions::default(), Vec::new(), owner)
            .await
            .unwrap();
        let mut wakes = own.wakes();
        let held = granted(
            own.take("proxy", &holder(&env, "own"), cap(1), now)
                .await
                .unwrap(),
        );
        own.take("proxy", &holder(&env, "own-waiting"), cap(1), now)
            .await
            .unwrap();

        own.give_back(held, cap(1), Ending::Done, now)
            .await
            .unwrap();

        received(&mut wakes)
    });

    assert_eq!(woken, vec!["own-waiting@proxy".to_owned()]);
}

/// Covers: story:3-81
#[test]
fn a_woken_waiter_its_own_cap_still_refuses_passes_the_wake_to_the_next() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let (strict, looser) = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let second = engine(&env, "second", owner).await;
        let mut wakes = second.wakes();
        let holding = holder(&env, "holding");
        let one = granted(
            first
                .take("shop.test", &holding, cap(2), now)
                .await
                .unwrap(),
        );
        let _two = granted(
            first
                .take("shop.test", &holding, cap(2), now)
                .await
                .unwrap(),
        );
        second
            .take("shop.test", &holder(&env, "strict"), cap(1), now)
            .await
            .unwrap();
        second
            .take("shop.test", &holder(&env, "looser"), cap(2), now)
            .await
            .unwrap();
        first
            .give_back(one, cap(2), Ending::Done, now)
            .await
            .unwrap();
        assert_eq!(next_wake(&mut wakes).await, "strict@shop.test");

        let strict = second
            .take("shop.test", &holder(&env, "strict"), cap(1), now)
            .await
            .unwrap();
        let passed = next_wake(&mut wakes).await;
        let looser = second
            .take("shop.test", &holder(&env, "looser"), cap(2), now)
            .await
            .unwrap();
        (strict, (passed, matches!(looser, Taken::Granted(_))))
    });

    assert_eq!(
        (strict, looser),
        (Taken::Wait(None), ("looser@shop.test".to_owned(), true))
    );
}

/// The run that gave a Permit back cannot take it again ahead of the
/// waiter it woke: the waiter's turn comes first.
///
/// Covers: story:3-80
#[test]
fn the_room_a_permit_given_back_frees_is_kept_for_the_waiter_it_woke() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let (again, waiter) = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let second = engine(&env, "second", owner).await;
        let mut wakes = second.wakes();
        let holding = holder(&env, "holding");
        let held = granted(
            first
                .take("shop.test", &holding, cap(1), now)
                .await
                .unwrap(),
        );
        second
            .take("shop.test", &holder(&env, "waiting"), cap(1), now)
            .await
            .unwrap();
        first
            .give_back(held, cap(1), Ending::Done, now)
            .await
            .unwrap();
        next_wake(&mut wakes).await;

        let again = first
            .take("shop.test", &holding, cap(1), now)
            .await
            .unwrap();
        let waiter = second
            .take("shop.test", &holder(&env, "waiting"), cap(1), now)
            .await
            .unwrap();
        (again, matches!(waiter, Taken::Granted(_)))
    });

    assert_eq!((again, waiter), (Taken::Wait(None), true));
}

/// Covers: story:3-80
#[test]
fn the_waiters_of_an_engine_that_ended_lose_their_turn_to_the_next() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let woken = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let ended_owner = TaskOwner::root(BlockingPolicy::Inline);
        let ended = engine(&env, "ended", &ended_owner).await;
        let last = engine(&env, "last", owner).await;
        let mut wakes = last.wakes();
        let held = granted(
            first
                .take("shop.test", &holder(&env, "holding"), cap(1), now)
                .await
                .unwrap(),
        );
        ended
            .take("shop.test", &holder(&env, "of-the-ended"), cap(1), now)
            .await
            .unwrap();
        last.take("shop.test", &holder(&env, "next"), cap(1), now)
            .await
            .unwrap();
        ended_owner.cancel();
        drop(ended);
        ended_owner.wait().await;

        first
            .give_back(held, cap(1), Ending::Done, now)
            .await
            .unwrap();
        next_wake(&mut wakes).await
    });

    assert_eq!(woken, "next@shop.test");
}

/// Covers: story:3-80
#[test]
fn the_permits_of_a_run_that_is_gone_are_reclaimed() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let (behind_live, behind_gone) = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let second = engine(&env, "second", owner).await;
        let _live = granted(
            first
                .take("live.test", &holder(&env, "live-run"), cap(1), now)
                .await
                .unwrap(),
        );
        let _gone = granted(
            first
                .take("gone.test", &holder(&env, "gone-run"), cap(1), now)
                .await
                .unwrap(),
        );
        let behind_live = second
            .take("live.test", &holder(&env, "other"), cap(1), now)
            .await
            .unwrap();
        let behind_gone = second
            .take("gone.test", &holder(&env, "other"), cap(1), now)
            .await
            .unwrap();
        (behind_live, matches!(behind_gone, Taken::Granted(_)))
    });

    assert_eq!((behind_live, behind_gone), (Taken::Wait(None), true));
}

/// Covers: story:3-95
#[test]
fn the_file_records_its_format_and_refuses_another_major() {
    let env = TestEnv::new();
    let path = env.home().join("politeness.sqlite");

    let recorded = run(async |owner| {
        engine(&env, "first", owner).await;
        Connection::open(&path)
            .unwrap()
            .query_row("SELECT name || ' ' || version FROM format", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
    });
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE format SET version = '1.0'", [])
        .unwrap();
    let refused = run(async |owner| {
        Politeness::open(
            path.clone(),
            &PolitenessOptions::default(),
            Vec::new(),
            gone_unless_live,
            "second".to_owned(),
            owner,
        )
        .await
        .err()
    });

    assert_eq!(
        (recorded, refused),
        (
            "politeness-file 0.1".to_owned(),
            Some(PolitenessError::Format {
                found: "1.0".to_owned(),
                compatibility: Compatibility::NewerMajor,
            })
        )
    );
}

/// Overwrites the pages of the table `table` and its indexes in the SQLite
/// file at `path`, its WAL first copied into it, with bytes that are no
/// B-tree page: SQLite reads them as it reads a page that met an I/O error.
fn damage_table(path: &Path, table: &str) {
    use std::io::{Seek as _, SeekFrom, Write as _};

    let connection = Connection::open(path).unwrap();
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap();
    let roots: Vec<i64> = connection
        .prepare("SELECT rootpage FROM sqlite_master WHERE tbl_name = ?1 AND rootpage > 0")
        .unwrap()
        .query_map([table], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(connection);
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    for root in roots {
        file.seek(SeekFrom::Start(
            u64::try_from((root - 1) * page_size).unwrap(),
        ))
        .unwrap();
        file.write_all(&vec![0xff; usize::try_from(page_size).unwrap()])
            .unwrap();
    }
}

/// SQLite reports a read that met an I/O error (EIO) as `SQLITE_CORRUPT`
/// (its Unix VFS's `SQLITE_IOERR_CORRUPTFS`): a take whose read meets one
/// fails with `Storage`, and the file's storage failures say so, as a full
/// disk's (ADR 0122).
///
/// Covers: row:RT-03
#[test]
fn a_take_whose_read_meets_a_damaged_disk_is_reported_as_a_storage_failure() {
    let env = TestEnv::new();
    let path = env.home().join("politeness.sqlite");
    run(async |owner| {
        engine(&env, "first", owner).await;
    });
    damage_table(&path, "permits");
    let now = SystemTime::now();

    let (failed, reported) = run(async |owner| {
        let politeness = engine(&env, "second", owner).await;
        let failed = politeness
            .take("shop.test", &holder(&env, "first"), cap(4), now)
            .await;
        (failed, politeness.storage_failures().borrow().is_some())
    });

    assert_eq!(
        (
            matches!(failed, Err(PolitenessError::Storage { .. })),
            reported
        ),
        (true, true),
        "{failed:?}"
    );
}
