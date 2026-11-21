//! The Usage ledger through its interface (spec #213's Seam 4), on real
//! SQLite in the test's folder: Slices taken, charged and given back, a
//! start refused once a Limit has nothing left, Slices reclaimed from dead
//! runs, the rows a newer engine or Foxhound wrote, and the format's
//! versions. The model-based test of crashes is in `model`.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_protocol::point_in_time;
use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, Row as CountedRow};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{LimitScope, Period, PeriodLength, ReachedLimit};
use goodfetch_protocol::usage::{Limit, LimitStanding};
use goodfetch_test_support::{TestEnv, damage_table, wal_of};
use jiff::Timestamp;
use rstest::rstest;

use super::{
    Charging, Holder, Ledger, LedgerError, Placement, Probed, Refusal, SliceId, SliceSizing,
};

mod allowance;
mod held_limits;
mod managed;
mod model;
mod reserve;
mod trace;

/// A moment of the tests' own: Tuesday 10 March 2026, 15:00 UTC.
fn now() -> Timestamp {
    "2026-03-10T15:00:00Z".parse().unwrap()
}

fn ledger(env: &TestEnv) -> Ledger {
    Ledger::at(env.home().join("usage-ledger.sqlite"), None)
}

fn install() -> LimitScope {
    LimitScope {
        kind: "install".to_owned(),
        id: String::new(),
    }
}

fn limit(scope: LimitScope, usage_kind: &str, amount: u64) -> Limit {
    Limit {
        scope,
        usage_kind: usage_kind.to_owned(),
        currency: None,
        period: None,
        amount,
        must_understand: true,
        set_by: LimitSetter::Person,
    }
}

/// A run of the install's, run by `product`'s engine.
fn placement(product: Product) -> Placement {
    Placement {
        product,
        scopes: vec![
            install(),
            LimitScope {
                kind: "person".to_owned(),
                id: "os_user".to_owned(),
            },
        ],
    }
}

fn holder(env: &TestEnv, run_id: &str) -> Holder {
    Holder {
        run_id: run_id.to_owned(),
        epoch: 1,
        folder: env.folder().join(run_id),
    }
}

fn charging(env: &TestEnv, run_id: &str) -> Charging {
    Charging::new(
        ledger(env),
        holder(env, run_id),
        placement(Product::Collie),
        SliceSizing::default(),
    )
}

/// Whether a new run of the install's, run by `product`'s engine, may
/// start at `at`.
fn may_start(env: &TestEnv, product: Product, at: Timestamp) -> Result<(), Refusal> {
    Charging::new(
        ledger(env),
        holder(env, "starting"),
        placement(product),
        SliceSizing::default(),
    )
    .may_start(at)
}

/// The install Limit's charged, held and left Usage now.
fn counted(ledger: &Ledger) -> (u64, u64, u64) {
    let usage = ledger.usage(Product::Collie, now()).unwrap();
    (usage[0].charged, usage[0].held, usage[0].left)
}

/// Covers: story:3-67, story:3-69
#[test]
fn a_run_holds_a_slice_while_it_charges_and_gives_back_what_it_did_not_use() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10_000), now())
        .unwrap();
    let mut run = charging(&env, "crawl");

    run.charge("requests", 3, now()).unwrap();
    let while_running = counted(&ledger);
    run.give_back().unwrap();

    assert_eq!(
        (while_running, counted(&ledger)),
        ((0, 100, 9_900), (3, 0, 9_997))
    );
}

/// Covers: story:3-69
#[test]
fn concurrent_runs_each_hold_slices_of_their_own() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut first = charging(&env, "first");
    first.charge("requests", 10, now()).unwrap();
    first.charge("requests", 1, now()).unwrap();

    let mut second = charging(&env, "second");
    second.charge("requests", 1, now()).unwrap();

    assert_eq!(counted(&ledger), (0, 30, 970));
}

/// Covers: story:3-71
#[test]
fn a_charge_beyond_what_is_left_is_refused_naming_the_limit() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 2), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 2, now()).unwrap();

    let refused = run.charge("requests", 1, now());

    assert_eq!(
        refused,
        Err(Refusal::Reached(Box::new(ReachedLimit {
            scope: install(),
            usage_kind: "requests".to_owned(),
            currency: None,
            period: None,
            resets_at: None,
        })))
    );
}

/// A run opens the ledger once, as it starts, for its Limits, all its
/// charges, the Slices they take, the Limits it reads back and its
/// give-back (#1886, E3-4: each Slice taken opened the file and prepared
/// its statements afresh; #1888: its start opened it once more).
#[test]
fn a_run_opens_the_ledger_once_for_its_charges_slices_reads_and_give_back() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut run = Charging::new(
        ledger.clone().counting(work.clone()),
        holder(&env, "crawl"),
        placement(Product::Collie),
        SliceSizing::default(),
    );
    run.may_start(now()).unwrap();
    let at_start = work.read(CountedRow::ChargeOpens);

    let slices: std::collections::BTreeSet<SliceId> = (0..40)
        .filter_map(|_| {
            run.charge("requests", 1, now())
                .unwrap()
                .used
                .first()
                .map(|(slice, _)| *slice)
        })
        .collect();
    run.allowed_by("requests", "USD").unwrap();
    run.applied(now()).unwrap();
    run.give_back().unwrap();
    let applied = run.applied(now()).unwrap();

    assert_eq!(
        (
            slices.len(),
            applied.len(),
            at_start,
            work.read(CountedRow::ChargeOpens),
            counted(&ledger)
        ),
        (4, 1, 1, 1, (40, 0, 960))
    );
}

/// A run's charge that fails at the ledger, here as it reads the Limits
/// before it takes a Slice, lets the run's connection go, and the next
/// opens the file afresh rather than reading on a connection a failure may
/// have left mid-transaction.
#[test]
fn a_runs_failed_charge_lets_its_connection_go() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut run = Charging::new(
        ledger.clone().counting(work.clone()),
        holder(&env, "crawl"),
        placement(Product::Collie),
        SliceSizing::default(),
    );
    run.charge("requests", 1, now()).unwrap();
    damage_table(ledger.path(), "limits").unwrap();

    let failed = [
        run.charge("requests", 100, now()),
        run.charge("requests", 100, now()),
    ];

    assert!(
        failed
            .iter()
            .all(|charged| matches!(charged, Err(Refusal::Ledger(LedgerError::Storage { .. })))),
        "{failed:?}"
    );
    assert_eq!(work.read(CountedRow::ChargeOpens), 2);
}

/// Covers: story:3-68, story:3-71
#[test]
fn a_new_run_is_refused_once_a_period_limit_has_nothing_left_until_its_next_period() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    let paris_day = Period {
        every: PeriodLength::Day,
        time_zone: "Europe/Paris".to_owned(),
    };
    ledger
        .add_limit(
            &Limit {
                period: Some(paris_day.clone()),
                ..limit(install(), "requests", 5)
            },
            now(),
        )
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 5, now()).unwrap();
    run.give_back().unwrap();

    let today = may_start(&env, Product::Collie, now());
    let tomorrow = may_start(
        &env,
        Product::Collie,
        "2026-03-10T23:30:00Z".parse().unwrap(),
    );

    assert_eq!(
        (today, tomorrow),
        (
            Err(Refusal::Reached(Box::new(ReachedLimit {
                scope: install(),
                usage_kind: "requests".to_owned(),
                currency: None,
                period: Some(paris_day),
                resets_at: Some(point_in_time::from_rfc3339("2026-03-10T23:00:00Z").unwrap()),
            }))),
            Ok(())
        )
    );
}

#[test]
fn a_limit_on_another_scope_never_holds_the_run() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    let other_person = LimitScope {
        kind: "person".to_owned(),
        id: "someone else".to_owned(),
    };
    ledger
        .add_limit(&limit(other_person, "requests", 0), now())
        .unwrap();

    assert_eq!(may_start(&env, Product::Collie, now()), Ok(()));
}

/// What a reclaimer finds of the dead run's state.
enum Found {
    Journaled(u64),
    Unreadable,
    Live,
}

/// Covers: story:3-72
#[rstest]
#[case::a_dead_run_is_charged_what_it_journaled(Found::Journaled(7), (7, 0, 9_993))]
#[case::a_dead_run_whose_state_is_unreadable_is_charged_its_whole_slice(Found::Unreadable, (100, 0, 9_900))]
#[case::a_live_run_keeps_its_slice(Found::Live, (0, 100, 9_900))]
fn an_engine_that_opens_the_ledger_reclaims_the_slices_of_dead_runs(
    #[case] found: Found,
    #[case] expected: (u64, u64, u64),
) {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10_000), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    let charged = run.charge("requests", 7, now()).unwrap();
    drop(run);

    ledger
        .reclaim(&|_| match found {
            Found::Journaled(used) => Probed::Gone {
                beyond: 0,
                used: charged
                    .used
                    .iter()
                    .map(|(slice, _)| (*slice, used))
                    .collect::<BTreeMap<_, _>>(),
            },
            Found::Unreadable => Probed::Unreadable,
            Found::Live => Probed::Live,
        })
        .unwrap();

    assert_eq!(counted(&ledger), expected);
}

#[test]
fn reclaiming_a_ledger_no_engine_created_leaves_it_uncreated() {
    let env = TestEnv::new();
    let ledger = ledger(&env);

    let reclaimed = ledger.reclaim(&|_| Probed::Unreadable).unwrap();

    assert_eq!(
        (reclaimed, ledger.path().exists()),
        (super::Reclaimed::default(), false)
    );
}

/// A run that no Limit holds reads the ledger as it starts and as it ends,
/// and never writes it: a ledger no engine created stays uncreated, so the
/// run syncs none of it (#2110).
#[test]
fn a_run_no_limit_holds_leaves_a_ledger_no_engine_created_uncreated() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    let mut run = charging(&env, "crawl");

    let may_start = run.may_start(now());
    let charged = run.charge("requests", 1, now()).unwrap();
    let applied = run.applied(now()).unwrap();
    run.give_back().unwrap();

    assert_eq!(
        (may_start, charged.used, applied, ledger.path().exists()),
        (Ok(()), Vec::new(), Vec::new(), false)
    );
}

/// A run's ledger syncs are its Slice's take and its give back: each
/// charge out of the Slice writes nothing, and closing
/// a connection leaves the synced write-ahead log as it is rather than
/// syncing it again into the file (#2110).
#[test]
fn a_run_syncs_the_ledger_once_to_take_its_slice_and_once_to_give_it_back() {
    let env = TestEnv::new();
    let name = env.unique_name("ledger-syncs");
    let vfs = goodfetch_test_support::CrashVfs::register(&name, env.seed()).unwrap();
    let ledger = Ledger::at(
        env.home().join("usage-ledger.sqlite"),
        Some(vfs.name().to_owned()),
    );
    ledger
        .add_limit(&limit(install(), "requests", 1_000_000), now())
        .unwrap();
    let before = vfs.syncs();
    let mut run = Charging::new(
        ledger.clone(),
        holder(&env, "crawl"),
        placement(Product::Collie),
        SliceSizing::default(),
    );

    run.may_start(now()).unwrap();
    for _ in 0..13 {
        run.charge("requests", 1, now()).unwrap();
    }
    let taken = vfs.syncs() - before;
    run.give_back().unwrap();
    drop(run);

    assert_eq!((taken, vfs.syncs() - before), (1, 2));
}

/// A Slice taken and given back stays so through a power cut once its
/// connection closed, though closing it no longer copies the write-ahead
/// log into the file: each commit synced it (ADR 0041, #2110).
#[rstest]
#[case::taken(false, (0, 10, 990))]
#[case::given_back(true, (4, 0, 996))]
fn a_slice_survives_a_power_cut_after_its_connection_closed(
    #[case] given_back: bool,
    #[case] expected: (u64, u64, u64),
) {
    let env = TestEnv::new();
    let name = env.unique_name("ledger-power-cut");
    let vfs = goodfetch_test_support::CrashVfs::register(&name, env.seed()).unwrap();
    let ledger = Ledger::at(
        env.home().join("usage-ledger.sqlite"),
        Some(vfs.name().to_owned()),
    );
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();
    let mut run = Charging::new(
        ledger.clone(),
        holder(&env, "crawl"),
        placement(Product::Collie),
        SliceSizing::default(),
    );
    run.charge("requests", 4, now()).unwrap();
    if given_back {
        run.give_back().unwrap();
    }
    drop(run);

    vfs.crash(goodfetch_test_support::CrashKind::PowerCut);
    goodfetch_test_support::after_crash(&vfs, &ledger.path().to_string_lossy()).unwrap();

    assert_eq!(counted(&ledger), expected);
}

#[test]
fn a_slice_reclaimed_before_its_run_gives_it_back_is_charged_once() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10_000), now())
        .unwrap();
    let mut run = charging(&env, "crawl");
    run.charge("requests", 7, now()).unwrap();
    ledger.reclaim(&|_| Probed::Unreadable).unwrap();

    run.give_back().unwrap();

    assert_eq!(counted(&ledger), (100, 0, 9_900));
}

/// A Limit as a newer engine, or Foxhound, wrote it.
fn written(scope_kind: &str, usage_kind: &str, time_zone: Option<&str>, flagged: bool) -> Limit {
    Limit {
        scope: LimitScope {
            kind: scope_kind.to_owned(),
            id: if scope_kind == "person" {
                "os_user".to_owned()
            } else {
                String::new()
            },
        },
        period: time_zone.map(|zone| Period {
            every: PeriodLength::Month,
            time_zone: zone.to_owned(),
        }),
        must_understand: flagged,
        ..limit(install(), usage_kind, 0)
    }
}

/// Covers: story:3-74
#[rstest]
#[case::an_unknown_scope_kind("galaxy", "requests", None)]
#[case::an_unknown_usage_kind_on_the_runs_scope("person", "teleports", None)]
#[case::an_unknown_time_zone("install", "requests", Some("Mars/Olympus_Mons"))]
fn a_limit_flagged_must_understand_that_the_engine_cannot_evaluate_refuses_runs(
    #[case] scope_kind: &str,
    #[case] usage_kind: &str,
    #[case] time_zone: Option<&str>,
) {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    let newer = written(scope_kind, usage_kind, time_zone, true);
    ledger.add_limit(&newer, now()).unwrap();

    let refused = may_start(&env, Product::Collie, now());

    assert_eq!(refused, Err(Refusal::NotUnderstood(Box::new(newer))));
}

#[rstest]
#[case::unflagged("galaxy", "requests", false)]
#[case::flagged_on_another_scope("daemon", "teleports", true)]
fn a_limit_the_engine_cannot_evaluate_is_ignored_unless_flagged_on_the_runs_scope(
    #[case] scope_kind: &str,
    #[case] usage_kind: &str,
    #[case] flagged: bool,
) {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&written(scope_kind, usage_kind, None, flagged), now())
        .unwrap();

    assert_eq!(may_start(&env, Product::Collie, now()), Ok(()));
}

/// Covers: story:3-67
#[rstest]
#[case::a_collie_engine_keeps_it_as_foxhounds(Product::Collie, LimitStanding::Foxhound, Ok(()))]
#[case::a_foxhound_engine_holds_its_runs_to_it(Product::Foxhound, LimitStanding::Held, Err(()))]
fn a_paid_fetch_limit_is_foxhounds(
    #[case] product: Product,
    #[case] standing: LimitStanding,
    #[case] starts: Result<(), ()>,
) {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(
            &Limit {
                currency: Some("USD".to_owned()),
                ..limit(install(), "paid_fetch", 0)
            },
            now(),
        )
        .unwrap();

    let listed = ledger.usage(product, now()).unwrap();
    let started = may_start(&env, product, now());

    assert_eq!(
        (listed[0].standing, started.map_err(drop)),
        (standing, starts)
    );
}

/// Writes a ledger whose format row says `version`.
fn ledger_at_version(env: &TestEnv, version: &str) -> Ledger {
    let ledger = ledger(env);
    ledger.usage(Product::Collie, now()).unwrap();
    let connection = rusqlite::Connection::open(ledger.path()).unwrap();
    connection
        .execute("UPDATE format SET version = ?1", [version])
        .unwrap();
    ledger
}

/// Covers: format:usage-ledger
#[test]
fn a_ledger_of_a_newer_minor_opens() {
    let env = TestEnv::new();
    let ledger = ledger_at_version(&env, "0.9");

    assert_eq!(ledger.limits(), Ok(Vec::new()));
}

#[test]
fn a_ledger_of_another_major_is_refused() {
    let env = TestEnv::new();
    let ledger = ledger_at_version(&env, "1.0");

    assert_eq!(
        ledger.limits(),
        Err(LedgerError::Format {
            found: "1.0".to_owned(),
            compatibility: Compatibility::NewerMajor,
        })
    );
}

/// A ledger of another major is refused before this engine writes to it:
/// the tables of this version it lacks are not created in it.
///
/// Covers: format:usage-ledger
#[test]
fn a_ledger_of_another_major_is_refused_without_being_written_to() {
    let env = TestEnv::new();
    let ledger = ledger_at_version(&env, "1.0");
    let connection = rusqlite::Connection::open(ledger.path()).unwrap();
    connection.execute_batch("DROP TABLE limits").unwrap();

    let refused = ledger.limits();

    let tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'limits'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (refused, tables),
        (
            Err(LedgerError::Format {
                found: "1.0".to_owned(),
                compatibility: Compatibility::NewerMajor,
            }),
            0
        )
    );
}

/// SQLite reports a read that met an I/O error (EIO) as `SQLITE_CORRUPT`
/// (its Unix VFS's `SQLITE_IOERR_CORRUPTFS`): a ledger read that meets one
/// fails as a storage failure, which stops every Run (ADR 0122), as the
/// run file's does.
///
/// Covers: story:3-67
#[test]
fn a_read_that_meets_a_damaged_disk_fails_as_a_storage_failure() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10), now())
        .unwrap();
    damage_table(ledger.path(), "limits").unwrap();

    let read = ledger.limits();

    assert!(matches!(read, Err(LedgerError::Storage { .. })), "{read:?}");
}

#[test]
fn a_ledger_keeps_the_columns_a_newer_minor_added() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.usage(Product::Collie, now()).unwrap();
    let connection = rusqlite::Connection::open(ledger.path()).unwrap();
    connection
        .execute_batch("ALTER TABLE limits ADD COLUMN note TEXT NOT NULL DEFAULT 'kept'")
        .unwrap();
    ledger
        .add_limit(&limit(install(), "requests", 10), now())
        .unwrap();

    let note: String = connection
        .query_row("SELECT note FROM limits", [], |row| row.get(0))
        .unwrap();

    assert_eq!(note, "kept");
}

/// Engines that open a new ledger at once, such as an engine and its
/// `usage` call at its start, each open it: switching the new file to WAL
/// waits for the others as every transaction does. Its 40 new ledgers lie
/// in memory, where it was red before the fix (047f3ab60): on a busy disk
/// their synced creations passed the Quick kill (fix-queue 123).
///
/// Covers: story:3-67
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test opens the ledger from threads of its own, as engines in their processes do"
)]
fn engines_opening_a_new_ledger_at_once_each_open_it() {
    let env = TestEnv::in_memory();
    let mut failures = Vec::new();
    for round in 0..40 {
        let path = env.home().join(format!("ledger-{round}.sqlite"));
        let opening: Vec<_> = (0..3)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    Ledger::at(path, None)
                        .add_limit(&limit(install(), "requests", 1), now())
                        .map(drop)
                })
            })
            .collect();
        failures.extend(
            opening
                .into_iter()
                .filter_map(|opening| opening.join().unwrap().err()),
        );
    }
    assert_eq!(failures, []);
}

/// The Usage ledger holds to the synced per-user store's durability: its WAL
/// stays beside it as a write's connection closes (#2110).
#[test]
fn the_wal_stays_beside_the_ledger_once_a_write_closed_its_connection() {
    let env = TestEnv::in_memory();
    let ledger = ledger(&env);

    ledger
        .add_limit(&limit(install(), "requests", 10), now())
        .unwrap();

    assert!(wal_of(ledger.path()).exists());
}
