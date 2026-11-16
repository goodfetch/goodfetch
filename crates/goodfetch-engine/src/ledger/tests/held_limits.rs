//! The Limits a run holds in memory (ADRs 0041 and 0140, #1888): a charge
//! its Slices cover runs no SQL and touches no file, the ledger's own
//! changes reach its next charge, whichever engine of the process made
//! them, and a ledger no engine created is not tried again at each charge.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_counted_work::{CountedWork, Row as CountedRow};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{Period, PeriodLength};
use goodfetch_protocol::usage::Limit;
use goodfetch_test_support::{CrashVfs, TestEnv};
use jiff::{SignedDuration, Timestamp};
use rstest::rstest;

use super::{charging, holder, install, limit, now, placement};
use crate::ledger::{Change, Charging, Ledger, Refusal, SliceSizing};

/// Which engine of the process sets a Limit while a run charges.
#[derive(Clone, Copy, Debug)]
enum Setter {
    /// The run's own engine, through the ledger its runs charge.
    ThisEngine,
    /// Another engine, through a ledger of its own on the same file.
    AnotherEngine,
}

/// The user's ledger on the crash VFS `vfs`, as one engine opens it.
fn ledger_on(env: &TestEnv, vfs: &CrashVfs) -> Ledger {
    Ledger::at(
        env.home().join("usage-ledger.sqlite"),
        Some(vfs.name().to_owned()),
    )
}

/// The ledger `setter`'s engine sets a Limit through.
fn ledger_of(setter: Setter, env: &TestEnv, vfs: &CrashVfs, own: &Ledger) -> Ledger {
    match setter {
        Setter::ThisEngine => own.clone(),
        Setter::AnotherEngine => ledger_on(env, vfs),
    }
}

fn run_on(env: &TestEnv, ledger: &Ledger) -> Charging {
    Charging::new(
        ledger.clone(),
        holder(env, "crawl"),
        placement(Product::Collie),
        SliceSizing::default(),
    )
}

/// Covers: story:3-67
#[test]
fn a_charge_the_held_slices_cover_runs_no_sql_and_no_file_operation() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-covered"), env.seed()).unwrap();
    let work = CountedWork::default();
    let ledger = ledger_on(&env, &vfs);
    ledger
        .add_limit(&limit(install(), "requests", 1_000_000), now())
        .unwrap();
    let mut run = run_on(&env, &ledger.clone().counting(work.clone()));
    run.charge("requests", 1, now()).unwrap();
    let before = (work.read(CountedRow::ChargeStatements), vfs.operations());

    for _ in 0..100 {
        run.charge("requests", 1, now()).unwrap();
    }

    assert_eq!(
        (
            work.read(CountedRow::ChargeStatements) - before.0,
            vfs.operations() - before.1
        ),
        (0, 0)
    );
}

/// Covers: story:3-71
#[rstest]
#[case::set_by_the_runs_own_engine(Setter::ThisEngine)]
#[case::set_by_another_engine(Setter::AnotherEngine)]
fn a_limit_set_through_the_ledger_holds_the_next_charge_of_a_run_that_holds_slices(
    #[case] setter: Setter,
) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-set"), env.seed()).unwrap();
    let ledger = ledger_on(&env, &vfs);
    ledger
        .add_limit(&limit(install(), "requests", 1_000_000), now())
        .unwrap();
    let mut run = run_on(&env, &ledger);
    run.charge("requests", 1, now()).unwrap();

    ledger_of(setter, &env, &vfs, &ledger)
        .add_limit(&limit(install(), "requests", 5), now())
        .unwrap();
    let next = run.charge("requests", 10, now());

    assert!(matches!(next, Err(Refusal::Reached(_))), "{next:?}");
}

/// Covers: story:3-71
#[test]
fn a_limit_raised_through_the_ledger_binds_the_next_charge() {
    let env = TestEnv::new();
    let ledger = super::ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 2), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 2, now()).unwrap();
    let before = run.charge("requests", 1, now());

    super::ledger(&env)
        .raise(&limit(install(), "requests", 10), now())
        .unwrap();
    let after = run.charge("requests", 1, now());

    assert_eq!(
        (
            matches!(before, Err(Refusal::Reached(_))),
            after.map(|charged| charged.used.len())
        ),
        (true, Ok(1))
    );
}

/// A run's Slices are its own, and a Limit whose remainder other runs'
/// Slices hold has nothing left for it once its own run out (ADR 0041).
///
/// Covers: story:3-69, story:3-71
#[test]
fn a_limit_reached_by_another_holders_slices_refuses_a_runs_next_slice() {
    let env = TestEnv::new();
    let ledger = super::ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 4), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 1, now()).unwrap();
    let mut other = charging(&env, "other");
    for _ in 0..3 {
        other.charge("requests", 1, now()).unwrap();
    }

    let next = run.charge("requests", 1, now());

    assert!(matches!(next, Err(Refusal::Reached(_))), "{next:?}");
}

/// The case of #2135: a run no Limit holds, on a machine whose ledger no
/// engine created, tried to open it at every charge.
#[test]
fn a_run_no_limit_holds_does_not_try_a_ledger_no_engine_created_at_each_charge() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-absent"), env.seed()).unwrap();
    let ledger = ledger_on(&env, &vfs);
    let mut run = run_on(&env, &ledger);
    run.may_start(now()).unwrap();
    let before = vfs.operations();

    for _ in 0..100 {
        run.charge("requests", 1, now()).unwrap();
    }

    assert_eq!(vfs.operations() - before, 0);
}

/// What the look for another process's changes costs such a run: one try
/// at the ledger each second of its engine's clock, however many charges
/// it makes meanwhile (ADR 0041).
#[test]
fn a_run_no_limit_holds_tries_a_ledger_no_engine_created_once_a_second_whatever_it_charges() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-second"), env.seed()).unwrap();
    let ledger = ledger_on(&env, &vfs);
    let mut run = run_on(&env, &ledger);
    let start = vfs.operations();
    run.may_start(now()).unwrap();
    let one_read = vfs.operations() - start;

    for second in 0..3 {
        for _ in 0..100 {
            run.charge("requests", 1, now() + SignedDuration::from_secs(second))
                .unwrap();
        }
    }

    assert_eq!(
        (one_read > 0, vfs.operations() - start),
        (true, 3 * one_read)
    );
}

/// Covers: story:3-71
#[rstest]
#[case::set_by_the_runs_own_engine(Setter::ThisEngine)]
#[case::set_by_another_engine(Setter::AnotherEngine)]
fn a_limit_set_in_a_ledger_no_engine_created_holds_the_next_charge(#[case] setter: Setter) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-created"), env.seed()).unwrap();
    let ledger = ledger_on(&env, &vfs);
    let mut run = run_on(&env, &ledger);
    run.may_start(now()).unwrap();
    run.charge("requests", 1, now()).unwrap();

    ledger_of(setter, &env, &vfs, &ledger)
        .add_limit(&limit(install(), "requests", 0), now())
        .unwrap();
    let next = run.charge("requests", 1, now());

    assert!(matches!(next, Err(Refusal::Reached(_))), "{next:?}");
}

/// An engine of another process changes the Limits without this process's
/// ledger seeing it, as a row written straight into the file is here: a run
/// reads its Limits again once a second of its engine's clock has passed,
/// so such a Limit holds it from then on (ADR 0041).
///
/// Covers: story:3-71
#[test]
fn a_limit_set_by_an_engine_of_another_process_holds_a_run_a_second_later() {
    let env = TestEnv::new();
    let ledger = super::ledger(&env);
    ledger
        .add_limit(&limit(install(), "paid_solves", 1), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.may_start(now()).unwrap();
    run.charge("requests", 1, now()).unwrap();

    rusqlite::Connection::open(ledger.path())
        .unwrap()
        .execute(
            "INSERT INTO limits (scope_kind, scope_id, usage_kind, amount, must_understand, set_by, set_at)
             VALUES ('install', '', 'requests', 0, 1, 'person', '2026-03-10T15:00:00Z')",
            [],
        )
        .unwrap();
    let later = run.charge("requests", 1, now() + SignedDuration::from_secs(1));

    assert!(matches!(later, Err(Refusal::Reached(_))), "{later:?}");
}

/// A run takes a Slice of the Limit as the ledger holds it then, not as the
/// run last read it (ADR 0041): a Limit an engine of another process raised,
/// as a row changed straight in the file is here, gives the run its next
/// Slice at once, with no second to wait.
///
/// Covers: story:3-71
#[test]
fn a_slice_is_taken_of_the_limit_as_the_ledger_holds_it_then() {
    let env = TestEnv::new();
    let ledger = super::ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 2), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 2, now()).unwrap();

    rusqlite::Connection::open(ledger.path())
        .unwrap()
        .execute("UPDATE limits SET amount = 10", [])
        .unwrap();
    let next = run.charge("requests", 1, now());

    assert_eq!(next.map(|charged| charged.used.len()), Ok(1));
}

/// A resumed Run reads its Limits as it resumes, as a new one does as it
/// starts, so its charges read nothing either; but whatever is left of
/// them, where a new Run is refused its start: a Limit with nothing left
/// refuses its next charge.
#[test]
fn a_resumed_run_reads_its_limits_as_it_resumes_and_a_limit_with_nothing_left_refuses_its_next_charge()
 {
    let env = TestEnv::new();
    let ledger = super::ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 1), now())
        .unwrap();
    let mut other = charging(&env, "other");
    other.charge("requests", 1, now()).unwrap();
    let mut run = charging(&env, "crawl");

    let resumed = run.resume(now());
    let in_memory = run.charge_held("paid_solves", 1, now());
    let next = run.charge("requests", 1, now());

    assert_eq!(
        (
            resumed,
            in_memory.is_some(),
            matches!(next, Err(Refusal::Reached(_)))
        ),
        (Ok(()), true, true)
    );
}

/// A run's Slices are of one period of their Limit: a charge after the
/// period ended comes out of a Slice of the next, though the run evaluates
/// a Limit's period once, not at each charge (E3-1 of the audit #1882).
///
/// Covers: story:3-68
#[test]
fn a_charge_after_its_limits_period_ended_takes_a_slice_of_the_next_period() {
    let env = TestEnv::new();
    let ledger = super::ledger(&env);
    ledger
        .add_limit(
            &Limit {
                period: Some(Period {
                    every: PeriodLength::Day,
                    time_zone: "Europe/Paris".to_owned(),
                }),
                ..limit(install(), "requests", 5)
            },
            now(),
        )
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 5, now()).unwrap();
    let tomorrow: Timestamp = "2026-03-10T23:30:00Z".parse().unwrap();

    let today = run.charge("requests", 1, now());
    let next_day = run.charge("requests", 5, tomorrow);

    assert_eq!(
        (
            matches!(today, Err(Refusal::Reached(_))),
            next_day.map(|charged| charged.used.len())
        ),
        (true, Ok(1))
    );
}

/// What a Run does with no blocking call (E3-3 of the audit #1882): a
/// charge its Slices cover, in memory, and nothing once the charge needs
/// the ledger, for a Slice or for Limits it has not read.
///
/// Covers: story:3-67
#[test]
fn a_charge_is_made_without_the_ledger_only_while_the_held_slices_cover_it() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-held"), env.seed()).unwrap();
    let ledger = ledger_on(&env, &vfs);
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut run = run_on(&env, &ledger);
    let unread = run.charge_held("requests", 1, now());
    run.charge("requests", 1, now()).unwrap();
    let before = vfs.operations();

    let covered = run.charge_held("requests", 9, now());
    let short = run.charge_held("requests", 1, now());

    assert_eq!(
        (
            unread,
            covered.map(|charged| charged.used),
            short,
            vfs.operations() - before
        ),
        (None, Some(vec![(1, 10)]), None, 0)
    );
}

/// A change of a run's charging that is a step in memory (issue 2603):
/// charges and reservations the Slices the run holds cover, while its
/// Limits are current, and their settling, reach neither SQL nor the
/// file. One that needs the ledger is not made in memory, and leaves
/// nothing changed once it gave back what it took on the way: no Slice
/// used, and the run not marked *uncapped* by a price it did not pay.
///
/// Covers: story:3-67
#[test]
fn a_change_the_held_slices_cover_is_made_in_memory_and_one_they_do_not_changes_nothing() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-change"), env.seed()).unwrap();
    let work = CountedWork::default();
    let ledger = ledger_on(&env, &vfs);
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut run = run_on(&env, &ledger.clone().counting(work.clone()));
    let unread = run
        .change_held(now(), |change| change.charge("requests", 1))
        .is_some();
    run.charge("requests", 1, now()).unwrap();
    let before = (work.read(CountedRow::ChargeStatements), vfs.operations());

    // Of the Slice of 10: 1 charged above, then 4, then 2 of 5 reserved.
    let covered = run
        .change_held(now(), |change| {
            let charged = change.charge("requests", 4)?;
            let (reservation, _) = change.reserve("requests", None, 5)?;
            let _ = change.settle(reservation, 2);
            Ok(charged.used)
        })
        .unwrap();
    // 3 are left: 2 reserved, then 2 more than the Slice has.
    let short = run
        .change_held(now(), |change| {
            let (reservation, _) = change.reserve("requests", None, 2)?;
            match change.charge("requests", 2) {
                Ok(charged) => Ok(charged),
                Err(unmade) => {
                    let _ = change.settle(reservation, 0);
                    Err(unmade)
                }
            }
        })
        .is_some();
    let left = run
        .change_held(now(), |change| change.charge("requests", 3))
        .map(|charged| charged.used);
    // A price no Limit caps, then a charge the full Slice does not cover.
    let unpaid = run
        .change_held(now(), |change| {
            let (reservation, _) = change.reserve("paid_solves", Some("USD"), 1_000)?;
            match change.charge("requests", 1) {
                Ok(charged) => Ok(charged),
                Err(unmade) => {
                    let _ = change.settle(reservation, 0);
                    Err(unmade)
                }
            }
        })
        .is_some();
    let still_capped = !run.uncapped();
    let uncapped = run
        .change_held(now(), |change| {
            let (reservation, _) = change.reserve("paid_solves", Some("USD"), 1_000)?;
            Ok(change.settle(reservation, 0))
        })
        .is_some()
        && run.uncapped();
    let asked = run
        .change_held(now(), |change| change.allowed_by("paid_solves", "USD"))
        .is_some();

    assert_eq!(
        (
            (
                unread,
                covered,
                short,
                left,
                (unpaid, still_capped),
                uncapped,
                asked
            ),
            work.read(CountedRow::ChargeStatements) - before.0,
            vfs.operations() - before.1
        ),
        (
            (
                false,
                vec![(1, 5)],
                false,
                Some(vec![(1, 10)]),
                (false, true),
                true,
                false
            ),
            0,
            0
        )
    );
}

/// A change stops being a step in memory once the run's Limits may no
/// longer be the ledger's: an engine of its process changed them. The
/// same change is then made through the ledger.
///
/// Covers: story:3-67
#[test]
fn a_change_is_no_step_in_memory_once_the_limits_changed() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-change-changed"), env.seed()).unwrap();
    let ledger = ledger_on(&env, &vfs);
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut run = run_on(&env, &ledger);
    run.charge("requests", 1, now()).unwrap();
    let one = |change: &mut Change<'_>| change.charge("requests", 1).map(|charged| charged.used);
    let before = run.change_held(now(), one);

    ledger
        .add_limit(&limit(install(), "paid_solves", 5), now())
        .unwrap();

    assert_eq!(
        (
            before,
            run.change_held(now(), one),
            run.change(now(), one).ok()
        ),
        (Some(vec![(1, 2)]), None, Some(vec![(1, 3)]))
    );
}
