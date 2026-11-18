//! `ROWS` and `NOT_RUN_YET` against the counters of
//! `goodfetch_counted_work::Row`, so the command's rows and the counted work
//! cannot drift apart.

use std::collections::BTreeSet;

use super::{NOT_RUN_YET, ROWS, UnderValgrind};

/// The rows whose figure a benchmark reads outside Counted work: Callgrind's
/// call counts of SQLite's prepare and of `CountedWork`'s constructor, its
/// instructions beneath the clean-up's removal, beneath a selector's
/// compilation and beneath the generic set's build, a counting VFS's syncs,
/// E6's sums over its off cell and over its cell of a Run's own Politeness,
/// its transactions in the cell at `politeness.delegation: false`,
/// the evidence rows' sum over the cell with learning off, the page waits at
/// `full`, which `F2.page_waits`' counter counts in the cells at that level,
/// and the lease reads past the memory budget, which `F3.lease_reads`'
/// counter counts in the cell at budget 0.
const READ_ELSEWHERE: [&str; 18] = [
    "E2.log_prepares",
    "E2.prepares",
    "E3.prepares",
    "E6.off_work",
    "E6.private_transactions",
    "E6.undelegated_transactions",
    "F1.clean_up_instructions",
    "F1.generic_set_instructions",
    "F1.selector_compilation_instructions",
    "F2.learning_off_evidence",
    "F2.page_waits.full",
    "F2.prepares",
    "F2.syncs",
    "F3.counted_work_constructions",
    "F3.lease_reads.spilled",
    "F3.prepares",
    "F3.prepares.hop",
    "F3.prepares.sitemap",
];

/// Every counter is a row of the command, counted or not run yet, and each
/// counted row with no counter is read under Valgrind or elsewhere.
#[test]
fn the_commands_rows_are_the_counters_and_the_figures_read_elsewhere() {
    let counters: BTreeSet<&str> = goodfetch_counted_work::Row::ALL
        .iter()
        .map(|row| row.name())
        .collect();
    let rows: BTreeSet<&str> = ROWS.iter().map(|row| row.name).collect();
    let not_run_yet: BTreeSet<&str> = NOT_RUN_YET.iter().map(|row| row.name).collect();
    let valgrind: BTreeSet<&str> = ROWS
        .iter()
        .filter(|row| row.valgrind.is_some())
        .map(|row| row.name)
        .collect();

    assert_eq!(
        rows.intersection(&not_run_yet).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "rows both counted and not run yet"
    );
    let listed: BTreeSet<&str> = rows.union(&not_run_yet).copied().collect();
    assert_eq!(
        counters.difference(&listed).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "counters no row of `xtask counts` holds"
    );
    let read_elsewhere: BTreeSet<&str> = valgrind.union(&READ_ELSEWHERE.into()).copied().collect();
    assert_eq!(
        rows.difference(&counters).copied().collect::<BTreeSet<_>>(),
        read_elsewhere,
        "counted rows with no counter, against those read under Valgrind or elsewhere"
    );
}

/// Under Valgrind a family's cells come from its example or from its
/// benchmark, never both, so no row of the other goes unread.
#[test]
fn a_family_reads_its_rows_under_valgrind_from_one_place() {
    let families = |example: bool| -> BTreeSet<&str> {
        ROWS.iter()
            .filter(|row| match row.valgrind {
                Some(UnderValgrind::Example) => example,
                Some(UnderValgrind::Benchmark(_)) => !example,
                None => false,
            })
            .map(|row| row.family)
            .collect()
    };
    let both: Vec<_> = families(true)
        .intersection(&families(false))
        .copied()
        .collect();
    assert!(both.is_empty(), "families read from both: {both:?}");
}
