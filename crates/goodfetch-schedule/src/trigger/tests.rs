use goodfetch_protocol::schedule::{EveryTrigger, ScheduleTrigger, TriggerInvalidReason};
use rstest::rstest;

use super::check;

fn every(every: u64) -> ScheduleTrigger {
    ScheduleTrigger::Every(EveryTrigger {
        every,
        anchor: None,
    })
}

#[rstest]
#[case::one_millisecond(1)]
#[case::fifteen_minutes(900_000)]
#[case::the_largest(u64::MAX)]
fn an_interval_of_at_least_one_millisecond_is_accepted(#[case] interval: u64) {
    assert_eq!(check(&every(interval)), Ok(()));
}

#[test]
fn an_interval_of_zero_is_out_of_range() {
    let refused = check(&every(0)).unwrap_err();

    assert_eq!(refused.details.reason, TriggerInvalidReason::OutOfRange);
    assert_eq!(refused.details.expression, "0");
}
