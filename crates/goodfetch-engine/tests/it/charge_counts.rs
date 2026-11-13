//! B3's charge scenario (ADR 0140), run as a test: each outcome counts its own
//! work, a plant adds exactly its units and no Limit takes the Slice's work
//! away. `xtask counts` holds the figures to the register, and `--prove` runs
//! the plants through it, `E3.prepares`' under Callgrind among them.

#[path = "../../examples/support/charge_counts.rs"]
mod scenario;

use scenario::{Counted, charges};

fn counted(limited: bool, planted: Option<&str>) -> Counted {
    let folder = tempfile::tempdir().unwrap();
    charges(folder.path(), limited, planted, None)
}

/// Covered charges and Slices taken are told apart: a covered charge opens
/// nothing and commits nothing, a Slice's transaction syncs, and the run's
/// ledger opens once for all of them (#1886).
#[test]
fn a_covered_charge_and_a_slice_taken_each_count_their_own_work() {
    let default = counted(true, None);

    assert_eq!((default.covered, default.slices), (36, 4));
    assert_eq!(default.statements, default.covered, "{default:?}");
    assert!(default.transactions >= default.slices, "{default:?}");
    assert!(default.syncs >= default.slices, "{default:?}");
    assert_eq!(default.opens, 1, "{default:?}");
}

/// Each row's plant adds one unit per its outcome, and nothing to the others'.
#[test]
fn a_plant_adds_one_unit_per_outcome_of_its_row() {
    let default = counted(true, None);

    let statements = counted(true, Some("E3.statements"));
    let transactions = counted(true, Some("E3.slice_transactions"));
    let opens = counted(true, Some("E3.opens"));

    assert_eq!(statements.statements - default.statements, default.covered);
    assert_eq!(
        transactions.transactions - default.transactions,
        default.slices
    );
    assert_eq!(
        opens.opens - default.opens,
        default.covered + default.slices
    );
}

/// With no Limit holding the run, no charge takes a Slice, so the Slice's
/// transactions and syncs go with it, and fewer connections open.
#[test]
fn no_limit_takes_the_slices_work_away() {
    let (default, off) = (counted(true, None), counted(false, None));

    assert_eq!((off.covered, off.slices), (40, 0));
    assert_eq!((off.transactions, off.syncs), (0, 0));
    assert!(off.opens < default.opens, "{off:?} against {default:?}");
}
