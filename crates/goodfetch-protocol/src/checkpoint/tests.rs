use super::RunStatePosition;

/// A later run epoch is ahead whatever the journal positions: a Checkpoint
/// of epoch 1 at position 9 is behind a run file of epoch 2 at position 1,
/// and within one epoch the journal position decides (ADR 0002's note of
/// #2322).
#[test]
fn positions_compare_by_run_epoch_then_journal_position() {
    let at = |run_epoch, journal_position| RunStatePosition {
        run_epoch,
        journal_position,
    };

    assert_eq!(
        (
            at(1, 9) < at(2, 1),
            at(2, 1) < at(2, 3),
            at(2, 3) == at(2, 3)
        ),
        (true, true, true)
    );
}
