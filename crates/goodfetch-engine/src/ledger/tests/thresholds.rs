//! Ledger threshold crossings, reclaim of unsent thresholds, custom threshold
//! settings, concurrent multi-engine deduplication, ledger-kept "Needs you"
//! items, and E3 Shared-path counts (ADRs 0041 and 0140; issue 233).

use std::sync::{Arc, Barrier};
use std::thread;

use goodfetch_counted_work::{CountedWork, Row as CountedRow};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{LimitScope, Period, PeriodLength};
use goodfetch_protocol::usage::Limit;
use goodfetch_test_support::{CrashVfs, TestEnv};

use super::{
    Charging, Holder, Ledger, Placement, Probed, SliceSizing, charging, holder, install, ledger,
    limit, now, placement,
};

/// A monthly install Limit of `amount` requests in UTC.
fn monthly_install(amount: u64) -> Limit {
    Limit {
        period: Some(Period {
            every: PeriodLength::Month,
            time_zone: "UTC".to_owned(),
        }),
        ..limit(install(), "requests", amount)
    }
}

/// Default thresholds (`50 %`, `80 %`, `100 %`) are recorded inside the Slice's
/// transaction and returned once each as Slices cross them, and reaching
/// `100 %` records one `limit_needs_you` item in the Usage ledger that resolves
/// once a person raises the Limit or its period restarts.
///
/// Covers: story:3-73, setting:usage.thresholds
#[test]
fn crossings_record_at_50_80_and_100_percent_and_100_percent_keeps_a_needs_you_item_in_the_ledger()
{
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger.add_limit(&monthly_install(10), now()).unwrap();
    let mut run = charging(&env, "crawl");

    let mut seen: Vec<(u64, u64)> = Vec::new();
    for step in 1..=10_u64 {
        let (charged, crossings) = run.charge_with_crossings("requests", 1, now());
        charged.unwrap();
        for crossing in crossings {
            seen.push((step, crossing.threshold));
            ledger.mark_threshold_sent(&crossing, now()).unwrap();
        }
    }

    assert_eq!(seen, vec![(5, 50), (8, 80), (10, 100)]);
    let items = ledger.needs_you_items(now()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].limit_amount, 10);
    assert_eq!(items[0].limit.scope, install());

    // Next month starts a fresh period: the old item resolves automatically.
    let next_month = "2026-04-01T00:00:00Z".parse().unwrap();
    assert_eq!(ledger.needs_you_items(next_month).unwrap().len(), 0);

    // Raising the Limit in the same period with room left also resolves it.
    ledger.raise(&monthly_install(20), now()).unwrap();
    assert_eq!(ledger.needs_you_items(now()).unwrap().len(), 0);
}

/// Custom `usage.thresholds` override the default notification percentages,
/// while reaching `100 %` still records the ledger-kept "Needs you" item.
///
/// Covers: story:3-73, setting:usage.thresholds
#[test]
fn custom_thresholds_override_notification_percentages_while_100_percent_still_records_needs_you() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10), now())
        .unwrap();
    let mut run = charging(&env, "custom").with_thresholds(vec![20, 70]);

    let mut thresholds = Vec::new();
    for _ in 0..10 {
        let (charged, crossings) = run.charge_with_crossings("requests", 1, now());
        charged.unwrap();
        for crossing in crossings {
            thresholds.push(crossing.threshold);
            ledger.mark_threshold_sent(&crossing, now()).unwrap();
        }
    }

    assert_eq!(thresholds, vec![20, 70]);
    assert_eq!(ledger.needs_you_items(now()).unwrap().len(), 1);
}

/// An engine that reclaims a dead engine's Slices receives any threshold
/// crossing the dead engine recorded in its Slice's transaction but never
/// marked sent; once marked sent, another reclaim sees none.
///
/// Covers: story:3-73
#[test]
fn reclaiming_a_dead_engines_slices_surfaces_unsent_thresholds_once() {
    let env = TestEnv::new();
    let ledger = ledger(&env);
    ledger
        .add_limit(&limit(install(), "requests", 10), now())
        .unwrap();

    let mut dead = charging(&env, "dead-run");
    let mut unsent = Vec::new();
    for _ in 0..5 {
        let (charged, crossings) = dead.charge_with_crossings("requests", 1, now());
        charged.unwrap();
        unsent.extend(crossings);
    }
    // Simulate crash: the dead engine recorded the 50 % crossing on Slice 5
    // but died before delivering and calling `mark_threshold_sent`.
    assert_eq!(unsent.len(), 1);
    assert_eq!(unsent[0].threshold, 50);
    drop(dead);

    let reclaimed = ledger.reclaim(&|_| Probed::Unreadable).unwrap();
    let thresholds: Vec<u64> = reclaimed
        .unsent_thresholds
        .iter()
        .map(|crossing| crossing.threshold)
        .collect();
    assert_eq!(thresholds, vec![50]);

    for crossing in &reclaimed.unsent_thresholds {
        ledger.mark_threshold_sent(crossing, now()).unwrap();
    }
    let second = ledger.reclaim(&|_| Probed::Unreadable).unwrap();
    assert_eq!(second.unsent_thresholds.len(), 0);
}

/// Three engines racing concurrently across the same Limit's thresholds record
/// and emit each threshold (`50 %`, `80 %`, `100 %`) exactly once across all
/// three engines combined (ADR 0041).
///
/// Covers: story:3-73
#[test]
fn three_engines_crossing_thresholds_together_emit_each_threshold_once() {
    let env = TestEnv::new();
    let ledger_path = env.home().join("usage-ledger.sqlite");
    let setup = Ledger::at(ledger_path.clone(), None);
    setup.add_limit(&monthly_install(30), now()).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..3)
        .map(|index| {
            let path = ledger_path.clone();
            let folder = env.folder().join(format!("engine-{index}"));
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let ledger = Ledger::at(path, None);
                let mut run = Charging::new(
                    ledger.clone(),
                    Holder {
                        run_id: format!("run-{index}"),
                        epoch: 1,
                        folder,
                    },
                    Placement {
                        product: Product::Collie,
                        scopes: vec![
                            LimitScope {
                                kind: "install".to_owned(),
                                id: String::new(),
                            },
                            LimitScope {
                                kind: "person".to_owned(),
                                id: "os_user".to_owned(),
                            },
                        ],
                    },
                    SliceSizing::default(),
                );
                let mut emitted = Vec::new();
                barrier.wait();
                loop {
                    let (charged, crossings) = run.charge_with_crossings("requests", 1, now());
                    if charged.is_err() {
                        break;
                    }
                    for crossing in crossings {
                        ledger.mark_threshold_sent(&crossing, now()).unwrap();
                        emitted.push(crossing.threshold);
                    }
                }
                emitted
            })
        })
        .collect();

    let mut all_emitted: Vec<u64> = handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect();
    all_emitted.sort_unstable();

    assert_eq!(all_emitted, vec![50, 80, 100]);
    assert_eq!(setup.needs_you_items(now()).unwrap().len(), 1);
}

/// Shared paths E3 (ADR 0140): a threshold crossing is recorded inside the
/// Slice's own single transaction and sync (`E3.slice_transactions = 1`), and
/// charges covered by held Slices execute zero SQL statements (`E3.statements = 0`).
///
/// Covers: story:3-73
#[test]
fn threshold_crossing_shares_the_slices_transaction_and_held_charges_run_no_sql() {
    let env = TestEnv::new();
    let vfs_name = env.unique_name("ledger-threshold-e3");
    let vfs = CrashVfs::register(&vfs_name, env.seed()).unwrap();
    let work = CountedWork::default();
    let ledger = Ledger::at(
        env.home().join("usage-ledger.sqlite"),
        Some(vfs.name().to_owned()),
    )
    .counting(work.clone());
    // Limit of 1_000 -> each Slice holds 10 requests (`SliceSizing::default`).
    ledger
        .add_limit(&limit(install(), "requests", 1_000), now())
        .unwrap();

    let mut run = Charging::new(
        ledger.clone(),
        holder(&env, "e3"),
        placement(Product::Collie),
        SliceSizing::default(),
    );
    run.may_start(now()).unwrap();

    // Spend 49 Slices (490 requests), leaving the 50th Slice to cross 50 %.
    let mut early = Vec::new();
    for _ in 0..49 {
        let (charged, crossings) = run.charge_with_crossings("requests", 10, now());
        charged.unwrap();
        early.extend(crossings);
    }
    assert_eq!(early.len(), 0);

    // The 50th Slice crosses 50 % inside its single transaction and sync.
    let tx_before = work.read(CountedRow::ChargeTransactions);
    let syncs_before = vfs.syncs();
    let (charged, crossings) = run.charge_with_crossings("requests", 1, now());
    charged.unwrap();
    assert_eq!(crossings.len(), 1);
    assert_eq!(crossings[0].threshold, 50);
    assert_eq!(work.read(CountedRow::ChargeTransactions) - tx_before, 1);
    assert_eq!(vfs.syncs() - syncs_before, 1);

    // Remaining 9 charges within the held Slice run zero SQL statements and
    // zero transactions.
    let stmts_before = work.read(CountedRow::ChargeStatements);
    let tx_held_before = work.read(CountedRow::ChargeTransactions);
    let opens_before = work.read(CountedRow::ChargeOpens);
    for _ in 0..9 {
        assert!(run.charge_held("requests", 1, now()).is_some());
    }
    assert_eq!(work.read(CountedRow::ChargeStatements) - stmts_before, 0);
    assert_eq!(
        work.read(CountedRow::ChargeTransactions) - tx_held_before,
        0
    );
    assert_eq!(work.read(CountedRow::ChargeOpens) - opens_before, 0);
}
