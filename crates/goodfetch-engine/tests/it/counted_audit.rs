//! B3's Audit entry scenario, which `examples/counts.rs` and `xtask counts`
//! read: the work it counts happens, an `audit.retention` adds no prune to
//! an append and a counting VFS sees the store's syncs (ADR 0140).

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_test_support::CountingVfs;

use crate::audit_scenario::{self as scenario, Audit, ENTRIES};

/// The E2 rows counted in process.
const ROWS: [Row; 7] = [
    Row::AuditConfigurationReads,
    Row::AuditLogOpens,
    Row::AuditLogTransactions,
    Row::AuditOpens,
    Row::AuditPrunes,
    Row::AuditTransactions,
    Row::AuditSyncs,
];

/// What one run counted of each of [`ROWS`], and the store's syncs.
fn run(audit: Audit) -> ([u64; 7], u64) {
    let home = tempfile::tempdir().unwrap();
    let vfs = CountingVfs::register();
    let work = CountedWork::default();
    let entries = scenario::run(home.path(), audit, Some(vfs.name()), &work, None);
    assert_eq!(entries, ENTRIES);
    (ROWS.map(|row| work.read(row)), vfs.syncs())
}

/// Each entry reads no settings file, its retention coming from the
/// engine's snapshot of them (#1885), commits its append and writes the
/// head file, which the store syncs; the store opens and commits its schema
/// set-up once, for every entry (#1886), counted once a log (#1887); a
/// retention adds nothing, the engine pruning as it starts and once a day
/// (#1887). `xtask counts --prove` plants each row's real work.
#[test]
fn an_audit_entry_counts_its_reads_opens_transactions_and_head_writes() {
    let (base, syncs) = run(Audit::Chained);
    assert_eq!(base, [0, 1, 2, 0, 0, ENTRIES, ENTRIES]);
    assert!(syncs >= ENTRIES, "{syncs}");

    let (unchained, _) = run(Audit::Unchained);
    assert_eq!(unchained, base);

    let (retained, _) = run(Audit::Retained);
    assert_eq!(retained, base);
}
