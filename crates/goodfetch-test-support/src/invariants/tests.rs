use rstest::rstest;

use super::{GuaranteeClass, Violation, ViolationKind, fencing, limits};
use crate::observation::{Epoch, ObservationLog, Operation, Record, ValueDigest};

mod group;

fn fence(epoch: Epoch) -> Record<()> {
    Record::new("engine", Operation::Fence { epoch })
}

fn write(epoch: Epoch, key: &str, value: &str) -> Record<()> {
    Record::new(
        "engine",
        Operation::Write {
            epoch,
            key: key.to_owned(),
            digest: ValueDigest::of(value),
        },
    )
}

fn commit(key: &str, value: &str) -> Record<()> {
    Record::new(
        "engine",
        Operation::Commit {
            key: key.to_owned(),
            digest: ValueDigest::of(value),
        },
    )
}

fn visible(epoch: Epoch, key: &str, value: &str) -> Record<()> {
    Record::new(
        "sink",
        Operation::Visible {
            epoch,
            key: key.to_owned(),
            digest: ValueDigest::of(value),
        },
    )
}

fn limit(usage: u64) -> Record<()> {
    Record::new(
        "lead",
        Operation::Limit {
            scope: "install/requests".to_owned(),
            usage,
        },
    )
}

fn reserve(scope: &str, usage: u64) -> Record<()> {
    Record::new(
        "engine",
        Operation::Reserve {
            scope: scope.to_owned(),
            usage,
        },
    )
}

fn settle(reserved: u64, charged: u64) -> Record<()> {
    Record::new(
        "engine",
        Operation::Settle {
            scope: "install/requests".to_owned(),
            reserved,
            charged,
        },
    )
}

fn log(records: &[Record<()>]) -> ObservationLog<()> {
    let mut log = ObservationLog::new();
    for record in records {
        log.push(record.clone());
    }
    log
}

/// A run under epoch 1 commits `r1`, crashes, and resumes under epoch 2,
/// which commits `r2`; the copy left under epoch 1 then tries `r3`.
fn resumed_run() -> Vec<Record<()>> {
    vec![
        fence(1),
        write(1, "r1", "a"),
        commit("r1", "a"),
        fence(2),
        write(1, "r3", "c"),
        write(2, "r2", "b"),
        commit("r2", "b"),
    ]
}

fn with(mut records: Vec<Record<()>>, more: &[Record<()>]) -> Vec<Record<()>> {
    records.extend_from_slice(more);
    records
}

#[rstest]
#[case::fenced_refusing_the_stale_copy(
    GuaranteeClass::Fenced,
    with(resumed_run(), &[visible(1, "r1", "a"), visible(2, "r2", "b")]),
)]
#[case::record_fenced_letting_the_stale_copy_add_a_record(
    GuaranteeClass::RecordFenced,
    with(resumed_run(), &[visible(1, "r1", "a"), visible(2, "r2", "b"), visible(1, "r3", "c")]),
)]
#[case::keyed_letting_a_stale_retry_land_on_its_key(
    GuaranteeClass::Keyed,
    with(resumed_run(), &[write(1, "r2", "b"), visible(1, "r1", "a"), visible(1, "r2", "b")]),
)]
#[case::at_least_once_holding_a_record_twice(
    GuaranteeClass::AtLeastOnce,
    with(resumed_run(), &[visible(1, "r1", "a"), visible(1, "r1", "a"), visible(2, "r2", "b")]),
)]
fn fencing_passes_a_delivery_its_guarantee_class_allows(
    #[case] class: GuaranteeClass,
    #[case] records: Vec<Record<()>>,
) {
    let log = log(&records);

    let verdict = fencing(&log, class);

    assert_eq!(verdict, Ok(()));
}

#[test]
fn fenced_delivery_fails_when_a_stale_record_is_visible() {
    let log = log(&with(
        resumed_run(),
        &[
            visible(1, "r1", "a"),
            visible(2, "r2", "b"),
            visible(1, "r3", "c"),
        ],
    ));

    let verdict = fencing(&log, GuaranteeClass::Fenced);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::StaleRecordVisible,
            records: vec![fence(2), write(1, "r3", "c"), visible(1, "r3", "c")],
        })
    );
}

#[test]
fn record_fenced_delivery_fails_when_a_stale_copy_overwrites_a_record() {
    let log = log(&with(
        resumed_run(),
        &[
            write(1, "r2", "z"),
            visible(1, "r1", "a"),
            visible(1, "r2", "z"),
        ],
    ));

    let verdict = fencing(&log, GuaranteeClass::RecordFenced);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::StaleOverwrite,
            records: vec![
                write(2, "r2", "b"),
                write(1, "r2", "z"),
                visible(1, "r2", "z")
            ],
        })
    );
}

#[test]
fn keyed_delivery_fails_when_a_key_is_visible_twice() {
    let log = log(&with(
        resumed_run(),
        &[
            visible(1, "r1", "a"),
            visible(2, "r2", "b"),
            visible(2, "r2", "b"),
        ],
    ));

    let verdict = fencing(&log, GuaranteeClass::Keyed);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::DuplicateKey,
            records: vec![visible(2, "r2", "b"), visible(2, "r2", "b")],
        })
    );
}

#[test]
fn at_least_once_delivery_fails_when_a_committed_record_is_missing() {
    let log = log(&with(resumed_run(), &[visible(1, "r1", "a")]));

    let verdict = fencing(&log, GuaranteeClass::AtLeastOnce);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::CommitMissing,
            records: vec![commit("r2", "b")],
        })
    );
}

/// The run of `resumed_run` as a real-process test logs it: the copy left
/// under epoch 1 runs in a process the test does not instrument, so its
/// writes are not in the log, nor are the crashed copy's.
fn resumed_run_without_writes() -> Vec<Record<()>> {
    vec![
        fence(1),
        commit("r1", "a"),
        fence(2),
        write(2, "r2", "b"),
        commit("r2", "b"),
    ]
}

#[test]
fn fenced_delivery_fails_when_a_stale_record_no_logged_write_explains_is_visible() {
    let log = log(&with(
        resumed_run_without_writes(),
        &[
            visible(1, "r1", "a"),
            visible(2, "r2", "b"),
            visible(1, "r3", "c"),
        ],
    ));

    let verdict = fencing(&log, GuaranteeClass::Fenced);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::StaleRecordVisible,
            records: vec![fence(2), visible(1, "r3", "c")],
        })
    );
}

#[test]
fn record_fenced_delivery_fails_when_an_unlogged_stale_copy_overwrites_a_record() {
    let log = log(&with(
        resumed_run_without_writes(),
        &[visible(1, "r1", "a"), visible(1, "r2", "z")],
    ));

    let verdict = fencing(&log, GuaranteeClass::RecordFenced);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::StaleOverwrite,
            records: vec![write(2, "r2", "b"), visible(1, "r2", "z")],
        })
    );
}

/// A record the log shows committed explains its visible copy, whoever
/// wrote it, and an unlogged stale copy may add a record under Record-fenced.
#[rstest]
#[case::fenced(
    GuaranteeClass::Fenced,
    with(resumed_run_without_writes(), &[visible(1, "r1", "a"), visible(2, "r2", "b")]),
)]
#[case::record_fenced(
    GuaranteeClass::RecordFenced,
    with(resumed_run_without_writes(), &[visible(1, "r1", "a"), visible(2, "r2", "b"), visible(1, "r3", "c")]),
)]
fn fencing_passes_an_unlogged_copy_a_commit_explains(
    #[case] class: GuaranteeClass,
    #[case] records: Vec<Record<()>>,
) {
    let log = log(&records);

    let verdict = fencing(&log, class);

    assert_eq!(verdict, Ok(()));
}

/// A Limit of 100 requests: two slices of 50, the first returned with 30
/// charged, then a slice of 20, which reaches the Limit exactly.
fn usage_up_to_the_limit() -> Vec<Record<()>> {
    vec![
        limit(100),
        reserve("install/requests", 50),
        reserve("install/requests", 50),
        settle(50, 30),
        reserve("person/requests", 500),
        reserve("install/requests", 20),
    ]
}

#[rstest]
#[case::reaching_the_limit(usage_up_to_the_limit(), 0)]
#[case::past_the_limit_by_the_allowed_loss(
    with(usage_up_to_the_limit(), &[reserve("install/requests", 1)]),
    1,
)]
fn limits_pass_usage_within_the_limit_and_the_allowed_loss(
    #[case] records: Vec<Record<()>>,
    #[case] allowed_loss: u64,
) {
    let log = log(&records);

    let verdict = limits(&log, allowed_loss);

    assert_eq!(verdict, Ok(()));
}

#[test]
fn limits_fail_when_usage_goes_past_the_limit_and_the_allowed_loss() {
    let log = log(&with(
        usage_up_to_the_limit(),
        &[reserve("install/requests", 2)],
    ));

    let verdict = limits(&log, 1);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::LimitExceeded,
            records: vec![
                limit(100),
                reserve("install/requests", 50),
                reserve("install/requests", 50),
                settle(50, 30),
                reserve("install/requests", 20),
                reserve("install/requests", 2),
            ],
        })
    );
}

/// Settling more Usage than is reserved is a bookkeeping error the checker
/// reports, rather than clamping it to zero, which would hide Usage a
/// reservation still holds.
#[test]
fn limits_fail_when_a_settle_returns_more_than_was_reserved() {
    let log = log(&[limit(100), reserve("install/requests", 10), settle(30, 10)]);

    let verdict = limits(&log, 0);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::OverSettled,
            records: vec![reserve("install/requests", 10), settle(30, 10)],
        })
    );
}
