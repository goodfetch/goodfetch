//! B3's Audit entry scenario, which `examples/counts.rs` and `xtask counts`
//! read: the work it counts happens, a plant adds one unit per entry, an
//! `audit.retention` prunes at each append and a counting VFS sees the
//! store's syncs (ADR 0140).

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_test_support::CountingVfs;

use crate::audit_scenario::{self as scenario, Audit, ENTRIES, Plant};

/// The E2 rows counted in process.
const ROWS: [Row; 5] = [
    Row::AuditConfigurationReads,
    Row::AuditOpens,
    Row::AuditPrunes,
    Row::AuditTransactions,
    Row::AuditSyncs,
];

/// What one run counted of each of [`ROWS`], and the store's syncs.
fn run(audit: Audit, planted: Option<Row>) -> ([u64; 5], u64) {
    let home = tempfile::tempdir().unwrap();
    let vfs = CountingVfs::register();
    let work = CountedWork::default();
    let plant = planted.map(|row| {
        let work = work.clone();
        Arc::new(
            move |_: &goodfetch_engine::audit::AuditLog, _: &tokio::runtime::Runtime| {
                work.count(row);
            },
        ) as Plant
    });
    let entries = scenario::run(home.path(), audit, Some(vfs.name()), &work, plant.as_ref());
    assert_eq!(entries, ENTRIES);
    (ROWS.map(|row| work.read(row)), vfs.syncs())
}

/// Each entry reads the settings files, commits its append and writes the
/// head file, which the store syncs; the store opens and commits its schema
/// set-up once, for every entry (#1886); a retention adds a prune pass per
/// entry, on the same connection; a plant adds one per entry.
#[test]
fn an_audit_entry_counts_its_reads_opens_transactions_and_head_writes_and_a_plant_adds_one_an_entry()
 {
    let ([reads, opens, prunes, transactions, head_writes], syncs) = run(Audit::Chained, None);
    assert_eq!(reads, ENTRIES);
    assert_eq!(opens, 1);
    assert_eq!(prunes, 0);
    assert!(
        (ENTRIES..=ENTRIES + 2).contains(&transactions),
        "{transactions}"
    );
    assert_eq!(head_writes, ENTRIES);
    assert!(syncs >= ENTRIES, "{syncs}");

    let (unchained, _) = run(Audit::Unchained, None);
    assert_eq!(unchained, [reads, opens, prunes, transactions, head_writes]);

    let ([_, retained_opens, retained_prunes, _, _], _) = run(Audit::Retained, None);
    assert_eq!(retained_prunes, ENTRIES);
    assert_eq!(retained_opens, opens);

    for (index, row) in ROWS.into_iter().enumerate() {
        let (planted, _) = run(Audit::Chained, Some(row));
        let base = [reads, opens, prunes, transactions, head_writes];
        assert_eq!(planted[index] - base[index], ENTRIES, "{}", row.name());
    }
}
