//! When a Schedule fires (ADR 0036): its trigger, checked when it is
//! created. This ticket's one trigger is a fixed interval, `every`, with an
//! optional anchor; its occurrences, and the cron and after-a-run triggers,
//! come with the scheduler.

use goodfetch_protocol::schedule::{
    EveryTrigger, ScheduleTrigger, ScheduleTriggerInvalidDetails, TriggerInvalidReason,
};

/// Why a trigger is refused, as `schedule_trigger_invalid` carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TriggerInvalid {
    /// What the call answers.
    pub(crate) details: ScheduleTriggerInvalidDetails,
    /// What it says, in the engine's English.
    pub(crate) message: String,
}

/// Checks `trigger`.
///
/// # Errors
///
/// [`TriggerInvalid`] for an `every` of 0, which would fire without end.
pub(crate) fn check(trigger: &ScheduleTrigger) -> Result<(), TriggerInvalid> {
    match trigger {
        ScheduleTrigger::Every(EveryTrigger { every: 0, .. }) => Err(TriggerInvalid {
            details: ScheduleTriggerInvalidDetails {
                expression: "0".to_owned(),
                position: 0,
                reason: TriggerInvalidReason::OutOfRange,
            },
            message: "The trigger's `every` is 0 ms; give an interval of at least 1 ms.".to_owned(),
        }),
        ScheduleTrigger::Every(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests;
