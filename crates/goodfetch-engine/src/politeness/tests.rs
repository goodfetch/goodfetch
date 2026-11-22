//! The Politeness file through its interface, on real SQLite in the test's
//! folder, with two engines' handles sharing one file (spec #213's Seam 4):
//! the combined count under a key against each run's own cap, waiters woken
//! in turn through their wake endpoints, a woken waiter still refused
//! passing the wake on, the Permits of a gone Run reclaimed, the file's
//! format, and a take whose transaction meets a full disk; and a Run's own
//! Politeness, held in memory, reserving a send at its grant.
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the wall clock as its rules' reading, and opens the file to look at it; its SQL statements are its own, on no Shared path (ADR 0140)"
)]

use std::path::Path;
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::home::POLITENESS_FILE;
use goodfetch_test_support::{CrashVfs, IoFault, TestEnv, damage_table};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _};
use tokio::sync::broadcast;

use super::{
    Ending, Hold, Holder, KeyState, MemoryTables, Permit, PermitRule, Politeness, PolitenessError,
    PolitenessOptions, PolitenessTables, SharedRule, Step, Taken, Verdict, Wake,
};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A run's own cap on the count in flight under a key, and nothing else.
struct Cap(u64);

/// The state of a rule that keeps none.
struct Stateless;

impl KeyState for Stateless {
    type Key = str;
    type Reads = ();
    type Writes = ();

    fn load(_: &Connection, _: &str, (): ()) -> rusqlite::Result<Self> {
        Ok(Self)
    }

    fn save(&self, _: &Connection, _: &str, (): &()) -> rusqlite::Result<()> {
        Ok(())
    }

    fn load_from(_: &MemoryTables, _: &str, (): ()) -> Self {
        Self
    }

    fn save_to(&self, _: &mut MemoryTables, _: &str, (): &()) {}
}

impl PermitRule for Cap {
    type State = Stateless;

    fn reads(&self, _: Step) {}

    fn admit(&self, _: &Stateless, in_flight: u64, _: SystemTime) -> Verdict {
        if in_flight < self.0 {
            Verdict::Go
        } else {
            Verdict::Busy
        }
    }

    fn started(&self, _: &mut Stateless, _: SystemTime) {}

    fn ended(
        &self,
        _: &mut Stateless,
        _: u64,
        _: Ending,
        _: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        (None, ())
    }
}

fn cap(cap: u64) -> SharedRule {
    SharedRule::new(Cap(cap))
}

/// A rule admitting every request whose sends it reserves for one moment.
struct Reserves(SystemTime);

impl PermitRule for Reserves {
    type State = Stateless;

    fn reads(&self, _: Step) {}

    fn admit(&self, _: &Stateless, _: u64, _: SystemTime) -> Verdict {
        Verdict::Go
    }

    fn started(&self, _: &mut Stateless, _: SystemTime) {}

    fn ended(
        &self,
        _: &mut Stateless,
        _: u64,
        _: Ending,
        _: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        (None, ())
    }

    fn reserved(&self, _: &mut Stateless, _: SystemTime) -> Option<(SystemTime, ())> {
        Some((self.0, ()))
    }
}

/// A Run of the test's: live unless its folder's name says it is gone, or
/// says it is dying and was looked at before.
fn holder(env: &TestEnv, run_id: &str) -> Holder {
    Holder {
        run_id: run_id.to_owned(),
        epoch: 1,
        run_folder: env.folder().join(run_id),
        delegates: true,
    }
}

fn gone_unless_live(folder: &Path, _epoch: u64) -> bool {
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.starts_with("gone") || (name.starts_with("dying") && !first_look(folder))
}

/// Whether `folder` is looked at for the first time: a dying Run ends
/// between its first look and the next.
fn first_look(folder: &Path) -> bool {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::File::create_new(folder.join("looked")).is_ok()
}

async fn engine(env: &TestEnv, name: &str, owner: &TaskOwner) -> Politeness {
    Politeness::open(
        env.home().join(POLITENESS_FILE),
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

/// Waits for the next wake `wakes` receives, at most [`WAKE_WITHIN`] of
/// real time. A wake crosses a real socket, or a named pipe on Windows, so
/// a blocking task holds the paused clock still meanwhile (TESTING.md,
/// Time): otherwise the runtime, idle while the wake is on its way, moves
/// the clock past the timeout before the wake arrives. The task lets the
/// clock go once the wake arrives or [`WAKE_WITHIN`] passes, and the
/// timeout then ends a wait no wake answered.
async fn next_wake(wakes: &mut broadcast::Receiver<Wake>) -> String {
    let (release, released) = std::sync::mpsc::channel::<()>();
    #[expect(
        clippy::disallowed_methods,
        reason = "holding the paused clock still is the blocking task's whole job (TESTING.md, Time)"
    )]
    let holding = tokio::task::spawn_blocking(move || released.recv_timeout(WAKE_WITHIN));
    let wake = tokio::time::timeout(WAKE_WITHIN, wakes.recv()).await;
    drop(release);
    holding.await.unwrap().unwrap_err();
    let wake = wake.expect("a wake arrives").unwrap();
    format!("{}@{}", wake.run_id, wake.key)
}

/// How long [`next_wake`] waits for a wake.
const WAKE_WITHIN: Duration = Duration::from_secs(10);

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

    let taken =
        crate::sqlite_wal::tests::opened_beside_a_write(&env.home().join(POLITENESS_FILE), || {
            run(async |owner| {
                let politeness = Politeness::open(
                    env.home().join(POLITENESS_FILE),
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
        });

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
    let file = env.home().join(POLITENESS_FILE);
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

    let (opened, held_throughout) = goodfetch_test_support::runtime_with_one_blocking_thread()
        .block_on(async {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let held = goodfetch_test_support::with_the_blocking_pool_held(Politeness::open(
                env.home().join(POLITENESS_FILE),
                &PolitenessOptions::default(),
                Vec::new(),
                gone_unless_live,
                "inline".to_owned(),
                &owner,
            ))
            .await;
            owner.cancel();
            held
        });

    assert_eq!(
        (
            opened.map(drop).map_err(|error| error.to_string()),
            held_throughout
        ),
        (Ok(()), true),
        "the open waited for the blocking pool"
    );
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
        let own = Politeness::in_memory(&PolitenessOptions::default(), owner).unwrap();
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

/// A take in turn on a Run's own Politeness, held in memory, gives its
/// Permit the moment the rule reserved its send at (#2171); a take beside
/// a sent request, and any take on the file, reserve nothing, their sends
/// asking the store.
///
/// Covers: story:5-90, setting:politeness.sharing
#[test]
fn only_a_take_in_turn_on_a_runs_own_politeness_reserves_its_send() {
    let env = TestEnv::new();
    let now = SystemTime::now();
    let goes = now + Duration::from_millis(250);
    let reserving_rule = || SharedRule::new(Reserves(goes));

    let reserved = run(async |owner| {
        let own = Politeness::in_memory(&PolitenessOptions::default(), owner).unwrap();
        let file = engine(&env, "file", owner).await;
        let take = granted(
            own.take("shop", &holder(&env, "own"), reserving_rule(), now)
                .await
                .unwrap(),
        );
        let beside = granted(
            own.take_beside("shop", &holder(&env, "own"), reserving_rule(), now)
                .await
                .unwrap(),
        );
        let on_file = granted(
            file.take("shop", &holder(&env, "shared"), reserving_rule(), now)
                .await
                .unwrap(),
        );
        (take.send_at(), beside.send_at(), on_file.send_at())
    });

    assert_eq!(reserved, (Some(goes), None, None));
}

/// A grant that keeps its holder's turn leaves the holder a waiter of the
/// key, ahead of one that waits after it: the Permit's give back wakes the
/// holder, not the later waiter, in the file and in memory alike, as a
/// grant followed by a refused take did (ADR 0053's turns, #1899).
///
/// Covers: story:3-81
#[test]
fn a_grant_that_keeps_its_turn_is_woken_ahead_of_a_later_waiter() {
    let env = TestEnv::new();
    let now = SystemTime::now();

    let woken = run(async |owner| {
        let first = engine(&env, "first", owner).await;
        let second = engine(&env, "second", owner).await;
        let own = Politeness::in_memory(&PolitenessOptions::default(), owner).unwrap();
        let mut first_wakes = first.wakes();
        let mut own_wakes = own.wakes();
        let mut woken = Vec::new();
        for (politeness, later) in [(&first, &second), (&own, &own)] {
            let kept = granted(
                politeness
                    .take_keeping_turn("shop.test", &holder(&env, "keeping"), cap(1), now)
                    .await
                    .unwrap(),
            );
            later
                .take("shop.test", &holder(&env, "later"), cap(1), now)
                .await
                .unwrap();
            politeness
                .give_back(kept, cap(1), Ending::Done, now)
                .await
                .unwrap();
        }
        woken.push(next_wake(&mut first_wakes).await);
        woken.extend(received(&mut own_wakes));
        woken
    });

    assert_eq!(
        woken,
        vec![
            "keeping@shop.test".to_owned(),
            "keeping@shop.test".to_owned()
        ]
    );
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
    let path = env.home().join(POLITENESS_FILE);

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

/// SQLite reports a read that met an I/O error (EIO) as `SQLITE_CORRUPT`
/// (its Unix VFS's `SQLITE_IOERR_CORRUPTFS`): a take whose read meets one
/// fails with `Storage`, and the file's storage failures say so, as a full
/// disk's (ADR 0122).
///
/// Covers: row:RT-03
#[test]
fn a_take_whose_read_meets_a_damaged_disk_is_reported_as_a_storage_failure() {
    let env = TestEnv::new();
    let path = env.home().join(POLITENESS_FILE);
    run(async |owner| {
        engine(&env, "first", owner).await;
    });
    damage_table(&path, "permits").unwrap();
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

/// The task that holds a Run's own Politeness runs every note its handles
/// sent before its own next take, first: a Permit given back with a
/// one-way note frees the key's cap of one for the take after it, with no
/// task between them (#2171).
///
/// Covers: story:5-90, setting:politeness.sharing
#[test]
fn a_runs_own_politeness_runs_its_handles_notes_before_its_holders_next_take() {
    let env = TestEnv::new();
    let now = SystemTime::now();
    let own_run = holder(&env, "own");

    let after_the_note = run(async |_| {
        let (politeness, mut own) = Politeness::own(&PolitenessOptions::default());
        let first = granted(own.take("shop", &own_run, &cap(1), now));
        politeness
            .note_given_back(first, cap(1), Ending::Done, now)
            .await
            .unwrap();
        own.take("shop", &own_run, &cap(1), now)
    });

    granted(after_the_note);
}

/// The task that holds a Run's own Politeness answers its handles' takes
/// as it serves them; once it is gone, a handle's take fails closed rather
/// than waiting for an answer (#2171).
///
/// Covers: story:5-90, setting:politeness.sharing
#[test]
fn a_runs_own_politeness_answers_its_handles_while_served_and_fails_closed_once_gone() {
    let env = TestEnv::new();
    let now = SystemTime::now();
    let own_run = holder(&env, "own");

    let (served, gone) = run(async |_| {
        let (politeness, mut own) = Politeness::own(&PolitenessOptions::default());
        let mut take = std::pin::pin!(politeness.take_beside("beside", &own_run, cap(1), now));
        let served = loop {
            tokio::select! {
                taken = &mut take => break taken,
                () = own.serve() => {}
            }
        };
        drop(own);
        let gone = politeness
            .take_beside("beside", &own_run, cap(1), now)
            .await;
        (served, gone)
    });

    granted(served.unwrap());
    assert!(matches!(gone, Err(PolitenessError::Closed)), "{gone:?}");
}

/// A message whose answer a handle of a Run's own Politeness waits for
/// counts once on `F3.politeness_read_round_trips` (option A of #2171); a
/// one-way note, a second copy's take and a read its caller counts apart
/// count none, and nothing counts there on the file.
///
/// Covers: story:5-90, setting:politeness.sharing
#[test]
fn only_an_answered_message_to_a_runs_own_politeness_counts_a_read_round_trip() {
    let env = TestEnv::new();
    let now = SystemTime::now();
    let own_work = CountedWork::default();
    let file_work = CountedWork::default();

    run(async |owner| {
        let own_options = PolitenessOptions {
            work: own_work.clone(),
            ..PolitenessOptions::default()
        };
        let own = Politeness::in_memory(&own_options, owner).unwrap();
        let file = Politeness::open(
            env.home().join(POLITENESS_FILE),
            &PolitenessOptions {
                work: file_work.clone(),
                ..PolitenessOptions::default()
            },
            Vec::new(),
            gone_unless_live,
            "file".to_owned(),
            owner,
        )
        .await
        .unwrap();
        for politeness in [&own, &file] {
            let run_holder = holder(&env, "run");
            granted(
                politeness
                    .take("shop", &run_holder, cap(2), now)
                    .await
                    .unwrap(),
            );
            granted(
                politeness
                    .take_beside("shop", &run_holder, cap(2), now)
                    .await
                    .unwrap(),
            );
            politeness
                .change("shop", (), |_: &mut Stateless| ((), ()))
                .await
                .unwrap();
            politeness
                .change_counted_apart("shop", (), |_: &mut Stateless| ((), ()))
                .await
                .unwrap();
            politeness
                .note("shop", (), |_: &mut Stateless| ())
                .await
                .unwrap();
        }
    });

    assert_eq!(
        (
            own_work.read(Row::PolitenessReadRoundTrips),
            file_work.read(Row::PolitenessReadRoundTrips)
        ),
        (2, 0)
    );
}

/// The start of the last request under each key, kept in the file and held
/// in memory by a key's Delegation (#2172).
struct LastStart(Option<i64>);

/// [`LastStart`] of each key a Delegation holds in memory.
#[derive(Default)]
struct HeldStarts(std::collections::HashMap<String, i64>);

const STARTS: PolitenessTables = PolitenessTables {
    name: "test-starts",
    statements: &[
        "CREATE TABLE IF NOT EXISTS test_starts (key TEXT NOT NULL PRIMARY KEY, at INTEGER NOT NULL) STRICT",
    ],
    prepared: &[
        "SELECT at FROM test_starts WHERE key = ?1",
        "INSERT OR REPLACE INTO test_starts (key, at) VALUES (?1, ?2)",
    ],
};

impl KeyState for LastStart {
    type Key = str;
    type Reads = ();
    type Writes = bool;

    const DELEGATES: bool = true;

    fn load(connection: &Connection, key: &str, (): ()) -> rusqlite::Result<Self> {
        connection
            .prepare_cached("SELECT at FROM test_starts WHERE key = ?1")?
            .query_row([key], |row| row.get(0))
            .optional()
            .map(Self)
    }

    fn save(&self, connection: &Connection, key: &str, wrote: &bool) -> rusqlite::Result<()> {
        if let (true, Some(at)) = (wrote, self.0) {
            connection
                .prepare_cached("INSERT OR REPLACE INTO test_starts (key, at) VALUES (?1, ?2)")?
                .execute((key, at))?;
        }
        Ok(())
    }

    fn load_from(tables: &MemoryTables, key: &str, (): ()) -> Self {
        Self(
            tables
                .get::<HeldStarts>()
                .and_then(|held| held.0.get(key).copied()),
        )
    }

    fn save_to(&self, tables: &mut MemoryTables, key: &str, wrote: &bool) {
        if let (true, Some(at)) = (wrote, self.0) {
            tables.get_mut::<HeldStarts>().0.insert(key.to_owned(), at);
        }
    }

    fn delegated_under(key: &str) -> Option<&str> {
        Some(key)
    }

    fn delegate(
        connection: &Connection,
        key: &str,
        tables: &mut MemoryTables,
    ) -> rusqlite::Result<()> {
        Self::load(connection, key, ())?.save_to(tables, key, &true);
        Ok(())
    }

    fn write_back(
        tables: &MemoryTables,
        key: &str,
        connection: &Connection,
    ) -> rusqlite::Result<()> {
        Self::load_from(tables, key, ()).save(connection, key, &true)
    }

    fn forget(tables: &mut MemoryTables, key: &str) {
        tables.get_mut::<HeldStarts>().0.remove(key);
    }
}

/// A cap on the count in flight under a key, and a delay between its
/// requests' starts.
struct Spaced {
    cap: u64,
    delay: Duration,
}

impl PermitRule for Spaced {
    type State = LastStart;

    fn reads(&self, _: Step) {}

    fn admit(&self, state: &LastStart, in_flight: u64, now: SystemTime) -> Verdict {
        let ready = state.0.map(|at| super::moment(at) + self.delay);
        match ready {
            Some(ready) if ready > now => Verdict::Until(ready, Hold::Delay),
            _ if in_flight >= self.cap => Verdict::Busy,
            _ => Verdict::Go,
        }
    }

    fn started(&self, state: &mut LastStart, now: SystemTime) -> bool {
        state.0 = Some(super::milliseconds(now));
        true
    }

    fn ended(
        &self,
        _: &mut LastStart,
        _: u64,
        _: Ending,
        _: SystemTime,
    ) -> (Option<SystemTime>, bool) {
        (None, false)
    }
}

fn spaced(cap: u64, delay: Duration) -> SharedRule {
    SharedRule::new(Spaced { cap, delay })
}

/// An engine of the test's whose file holds [`LastStart`], counting its
/// work in `work`.
async fn delegating_engine(
    env: &TestEnv,
    name: &str,
    work: &CountedWork,
    owner: &TaskOwner,
) -> Politeness {
    Politeness::open(
        env.home().join(POLITENESS_FILE),
        &PolitenessOptions {
            work: work.clone(),
            ..PolitenessOptions::default()
        },
        vec![STARTS],
        gone_unless_live,
        name.to_owned(),
        owner,
    )
    .await
    .unwrap()
}

/// A moment a whole number of milliseconds after the Unix epoch, as the
/// file keeps it.
fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
}

/// The transactions of `work`'s file: a Permit's, a Delegation's, and
/// the others, a Run's own and an origin's; and how many Delegations it
/// took.
fn transactions(work: &CountedWork) -> [u64; 4] {
    [
        work.read(Row::PolitenessTransactions),
        work.read(Row::PolitenessDelegationTransactions),
        work.read(Row::PolitenessRunTransactions) + work.read(Row::PolitenessOriginTransactions),
        work.read(Row::PolitenessDelegations),
    ]
}

/// What `work` counted on a Run's own transactions and on an origin's,
/// after `rounds` Permits of one key at `politeness.delegation` set to
/// `delegates`, in a home of its own.
fn own_and_origin_transactions(delegates: bool, rounds_taken: u64) -> (u64, u64) {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let crawl = Holder {
        delegates,
        ..holder(&env, "crawl")
    };
    run(async |owner| {
        let politeness = delegating_engine(&env, "first", &work, owner).await;
        rounds(&politeness, &crawl, "shop.test", rounds_taken).await;
    });
    (
        work.read(Row::PolitenessRunTransactions),
        work.read(Row::PolitenessOriginTransactions),
    )
}

/// A Run's own transactions on the file, its open, its stop and the
/// Delegations it gives back, are the Run's whatever its attempts: as
/// many for one Permit as for five, with or without a Delegation, and
/// none on an origin's row when no site asked anything (#1899).
#[test]
fn a_runs_own_transactions_do_not_grow_with_its_attempts() {
    for delegates in [true, false] {
        let (one, five) = (
            own_and_origin_transactions(delegates, 1),
            own_and_origin_transactions(delegates, 5),
        );

        assert_eq!(one, five, "at delegation {delegates}");
        assert!(one.0 > 0, "at delegation {delegates}: no Run transaction");
        assert_eq!(one.1, 0, "at delegation {delegates}: an origin's");
    }
}

/// Takes and gives back `rounds` Permits of `key` for `holder`, one delay
/// apart, then stops `holder`'s Run.
async fn rounds(politeness: &Politeness, holder: &Holder, key: &str, rounds: u64) {
    let rule = spaced(1, Duration::from_secs(1));
    for round in 0..rounds {
        let permit = granted(
            politeness
                .take(key, holder, rule.clone(), at(round))
                .await
                .unwrap(),
        );
        politeness
            .give_back(permit, rule.clone(), Ending::Done, at(round))
            .await
            .unwrap();
    }
    politeness.stop_waiting(holder, at(rounds)).await.unwrap();
}

/// The start the file keeps for `key`.
fn kept_start(env: &TestEnv, key: &str) -> Option<i64> {
    let file = Connection::open(env.home().join(POLITENESS_FILE)).unwrap();
    file.query_row("SELECT at FROM test_starts WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .unwrap()
}

/// A Run at the default `politeness.delegation` takes its key's Delegation
/// in one transaction and serves its Permits from memory, with no
/// transaction per Permit; its state reaches the file as its Run stops and
/// gives the Delegation back (ADR 0142).
///
/// Covers: setting:politeness.delegation
#[test]
fn a_delegating_run_takes_its_permits_with_no_transaction_per_permit() {
    let env = TestEnv::new();
    let work = CountedWork::default();

    run(async |owner| {
        let politeness = delegating_engine(&env, "first", &work, owner).await;
        rounds(&politeness, &holder(&env, "crawl"), "shop.test", 3).await;
    });

    assert_eq!(
        transactions(&work)[..2],
        [0, 1],
        "Permit and Delegation transactions"
    );
    assert_eq!(transactions(&work)[3], 1, "Delegations");
    assert_eq!(
        kept_start(&env, "shop.test"),
        Some(super::milliseconds(at(2)))
    );
}

/// ADR 0142's Nothing-removed row: a Run at `politeness.delegation =
/// false` takes and gives back each Permit in a transaction of the file,
/// and takes no Delegation.
///
/// Covers: setting:politeness.delegation
#[test]
fn a_run_at_delegation_false_takes_each_permit_in_a_transaction_of_the_file() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let apart = Holder {
        delegates: false,
        ..holder(&env, "crawl")
    };

    run(async |owner| {
        let politeness = delegating_engine(&env, "first", &work, owner).await;
        rounds(&politeness, &apart, "shop.test", 3).await;
    });

    assert_eq!(
        transactions(&work)[..2],
        [6, 0],
        "Permit and Delegation transactions"
    );
    assert_eq!(transactions(&work)[3], 0, "Delegations");
    assert_eq!(
        kept_start(&env, "shop.test"),
        Some(super::milliseconds(at(2)))
    );
}

/// Two Runs of one engine share its Delegation of a key: one's Permit in
/// flight holds the other at the cap, and its give back wakes the other
/// within the engine, with no transaction.
///
/// Covers: setting:politeness.delegation
#[test]
fn two_runs_of_one_engine_keep_a_delegated_keys_cap_together() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let rule = spaced(1, Duration::ZERO);

    let (refused, woken, regranted) = run(async |owner| {
        let politeness = delegating_engine(&env, "first", &work, owner).await;
        let mut wakes = politeness.wakes();
        let (first, second) = (holder(&env, "first-run"), holder(&env, "second-run"));
        let permit = granted(
            politeness
                .take("shop.test", &first, rule.clone(), at(0))
                .await
                .unwrap(),
        );
        let refused = politeness
            .take("shop.test", &second, rule.clone(), at(0))
            .await
            .unwrap();
        politeness
            .give_back(permit, rule.clone(), Ending::Done, at(1))
            .await
            .unwrap();
        let woken = next_wake(&mut wakes).await;
        let regranted = politeness
            .take("shop.test", &second, rule.clone(), at(1))
            .await
            .unwrap();
        (refused, woken, matches!(regranted, Taken::Granted(_)))
    });

    assert_eq!(
        (refused, woken.as_str(), regranted),
        (Taken::Wait(None), "second-run@shop.test", true)
    );
    assert_eq!(
        transactions(&work)[..2],
        [0, 1],
        "Permit and Delegation transactions"
    );
}

/// Two Runs of one engine share its Delegation's delay too: the second
/// Run's take waits out the delay counted from the first Run's start.
///
/// Covers: setting:politeness.delegation
#[test]
fn two_runs_of_one_engine_keep_a_delegated_keys_delay_together() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let delay = Duration::from_secs(10);
    let rule = spaced(4, delay);

    let (held, ready) = run(async |owner| {
        let politeness = delegating_engine(&env, "first", &work, owner).await;
        let (first, second) = (holder(&env, "first-run"), holder(&env, "second-run"));
        granted(
            politeness
                .take("shop.test", &first, rule.clone(), at(0))
                .await
                .unwrap(),
        );
        let held = politeness
            .take("shop.test", &second, rule.clone(), at(1))
            .await
            .unwrap();
        let ready = politeness
            .take("shop.test", &second, rule.clone(), at(0) + delay)
            .await
            .unwrap();
        (held, matches!(ready, Taken::Granted(_)))
    });

    assert_eq!(
        (held, ready),
        (Taken::Wait(Some((at(0) + delay, Hold::Delay))), true)
    );
    assert_eq!(transactions(&work)[0], 0, "Permit transactions");
}

/// Another engine's take of a key whose Delegation a live Run holds waits
/// in turn; one whose holder is gone reclaims it, and the key's delay
/// counts from the reclaim, as the gone Run may have sent a request then.
///
/// Covers: setting:politeness.delegation
#[test]
fn a_gone_holders_delegation_is_reclaimed_with_the_keys_delay_from_the_reclaim() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let delay = Duration::from_secs(10);
    let rule = spaced(4, delay);

    let (held, reclaimed) = run(async |owner| {
        let first = delegating_engine(&env, "first", &work, owner).await;
        let second = delegating_engine(&env, "second", &CountedWork::default(), owner).await;
        granted(
            first
                .take("live.test", &holder(&env, "live"), rule.clone(), at(0))
                .await
                .unwrap(),
        );
        granted(
            first
                .take("gone.test", &holder(&env, "gone-run"), rule.clone(), at(0))
                .await
                .unwrap(),
        );
        let held = second
            .take("live.test", &holder(&env, "other"), rule.clone(), at(20))
            .await
            .unwrap();
        let reclaimed = second
            .take("gone.test", &holder(&env, "other"), rule.clone(), at(20))
            .await
            .unwrap();
        (held, reclaimed)
    });

    assert_eq!(held, Taken::Wait(None));
    assert_eq!(reclaimed, Taken::Wait(Some((at(20) + delay, Hold::Delay))));
}

/// Another engine's take of a key whose Delegation a live Run holds
/// recalls it: the holder writes its Permit in flight to the file and
/// wakes the waiter, whose retake still finds the cap held; the holder's
/// give back, now in the file, wakes it again and its take is granted.
///
/// Covers: setting:politeness.delegation
#[test]
fn another_engines_take_recalls_the_delegation_with_its_permits_in_flight() {
    let env = TestEnv::new();
    let rule = spaced(1, Duration::ZERO);

    let answers = run(async |owner| {
        let first = delegating_engine(&env, "first", &CountedWork::default(), owner).await;
        let second = delegating_engine(&env, "second", &CountedWork::default(), owner).await;
        let mut wakes = second.wakes();
        let (holding, other) = (holder(&env, "holding"), holder(&env, "other"));
        let permit = granted(
            first
                .take("shop.test", &holding, rule.clone(), at(0))
                .await
                .unwrap(),
        );
        let held = second
            .take("shop.test", &other, rule.clone(), at(1))
            .await
            .unwrap();
        let recalled = next_wake(&mut wakes).await;
        let still_held = second
            .take("shop.test", &other, rule.clone(), at(1))
            .await
            .unwrap();
        first
            .give_back(permit, rule.clone(), Ending::Done, at(2))
            .await
            .unwrap();
        let woken = next_wake(&mut wakes).await;
        let granted_after = second
            .take("shop.test", &other, rule.clone(), at(2))
            .await
            .unwrap();
        (
            held,
            recalled,
            still_held,
            woken,
            matches!(granted_after, Taken::Granted(_)),
        )
    });

    assert_eq!(
        answers,
        (
            Taken::Wait(None),
            "other@shop.test".to_owned(),
            Taken::Wait(None),
            "other@shop.test".to_owned(),
            true
        )
    );
}

/// A Run of the holder's engine that waited for the key before the recall
/// keeps its turn ahead of the engine that recalled it: the holder's give
/// back wakes it, and its take is granted while the recaller still waits.
///
/// Covers: setting:politeness.delegation
#[test]
fn a_recalled_delegations_waiters_keep_their_turn_ahead_of_the_recaller() {
    let env = TestEnv::new();
    let rule = spaced(1, Duration::ZERO);

    let answers = run(async |owner| {
        let first = delegating_engine(&env, "first", &CountedWork::default(), owner).await;
        let second = delegating_engine(&env, "second", &CountedWork::default(), owner).await;
        let (mut first_wakes, mut second_wakes) = (first.wakes(), second.wakes());
        let (holding, behind, other) = (
            holder(&env, "holding"),
            holder(&env, "behind"),
            holder(&env, "other"),
        );
        let permit = granted(
            first
                .take("shop.test", &holding, rule.clone(), at(0))
                .await
                .unwrap(),
        );
        let waits = first
            .take("shop.test", &behind, rule.clone(), at(0))
            .await
            .unwrap();
        second
            .take("shop.test", &other, rule.clone(), at(1))
            .await
            .unwrap();
        next_wake(&mut second_wakes).await;
        second
            .take("shop.test", &other, rule.clone(), at(1))
            .await
            .unwrap();
        first
            .give_back(permit, rule.clone(), Ending::Done, at(2))
            .await
            .unwrap();
        let woken = next_wake(&mut first_wakes).await;
        let behind_takes = first
            .take("shop.test", &behind, rule.clone(), at(2))
            .await
            .unwrap();
        let other_takes = second
            .take("shop.test", &other, rule.clone(), at(2))
            .await
            .unwrap();
        (
            waits,
            woken,
            matches!(behind_takes, Taken::Granted(_)),
            other_takes,
        )
    });

    assert_eq!(
        answers,
        (
            Taken::Wait(None),
            "behind@shop.test".to_owned(),
            true,
            Taken::Wait(None)
        )
    );
}

/// A recall that finds its holder's engine gone falls back to the
/// dead-holder reclaim: the recaller is woken at once, and its retake
/// reclaims the Delegation of the Run that ended, the key's delay
/// counted from the reclaim, rather than waiting for its own deadline.
///
/// Covers: setting:politeness.delegation
#[test]
fn a_recall_to_a_holder_that_is_gone_falls_back_to_the_dead_holder_reclaim() {
    let env = TestEnv::new();
    let delay = Duration::from_secs(10);
    let rule = spaced(1, delay);

    let answers = run(async |owner| {
        let ended_owner = TaskOwner::root(BlockingPolicy::Inline);
        let first = delegating_engine(&env, "first", &CountedWork::default(), &ended_owner).await;
        let second = delegating_engine(&env, "second", &CountedWork::default(), owner).await;
        let mut wakes = second.wakes();
        let other = Holder {
            delegates: false,
            ..holder(&env, "other")
        };
        granted(
            first
                .take("shop.test", &holder(&env, "dying"), rule.clone(), at(0))
                .await
                .unwrap(),
        );
        ended_owner.cancel();
        drop(first);
        ended_owner.wait().await;

        let held = second
            .take("shop.test", &other, rule.clone(), at(1))
            .await
            .unwrap();
        let woken = next_wake(&mut wakes).await;
        let retaken = second
            .take("shop.test", &other, rule.clone(), at(1))
            .await
            .unwrap();
        (held, woken, retaken)
    });

    assert_eq!(
        answers,
        (
            Taken::Wait(None),
            "other@shop.test".to_owned(),
            Taken::Wait(Some((at(1) + delay, Hold::Delay)))
        )
    );
}

/// The takes of many keys that reach the engine together take their
/// Delegations together: one transaction for the group.
///
/// Covers: setting:politeness.delegation
#[test]
fn takes_of_many_keys_take_their_delegations_in_shared_transactions() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let rule = spaced(1, Duration::ZERO);
    let keys: Vec<String> = (0..50).map(|key| format!("site-{key}.test")).collect();

    let granted_all = run(async |owner| {
        let politeness = delegating_engine(&env, "first", &work, owner).await;
        let crawl = holder(&env, "crawl");
        let takes = keys
            .iter()
            .map(|key| politeness.take(key, &crawl, rule.clone(), at(0)));
        futures_util::future::join_all(takes)
            .await
            .into_iter()
            .all(|taken| matches!(taken, Ok(Taken::Granted(_))))
    });

    let [permits, delegating, _, delegations] = transactions(&work);
    assert!(granted_all);
    assert_eq!((permits, delegating, delegations), (0, 1, 50));
}
