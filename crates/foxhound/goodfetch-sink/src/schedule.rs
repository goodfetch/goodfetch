//! The delivery retry schedule (ADRs 0026 and 0129): a pure function of the
//! `Clock`'s reading (ADR 0059). Retries follow the Standard Webhooks schedule
//! (immediately, 5 s, 5 min, 30 min, 2 h, 5 h, 10 h, 14 h, 20 h, 24 h),
//! honouring `Retry-After`, and pause the Sink after three days of total
//! failure (or immediately on HTTP 410).

use std::time::{Duration, SystemTime};

/// How long after each failed attempt the next one waits: the Standard
/// Webhooks schedule (ADR 0026), whose first attempt is immediate.
pub const SCHEDULE: [Duration; 9] = [
    Duration::from_secs(5),
    Duration::from_mins(5),
    Duration::from_mins(30),
    Duration::from_hours(2),
    Duration::from_hours(5),
    Duration::from_hours(10),
    Duration::from_hours(14),
    Duration::from_hours(20),
    Duration::from_hours(24),
];

/// Three days of total failure pause the Sink with a Notification (ADR 0026).
pub const PAUSE_AFTER: Duration = Duration::from_hours(72);

/// Why a Sink was paused (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    /// The destination answered HTTP 410 Gone.
    Gone,
    /// The Sink failed continuously for three days.
    ThreeDaysFailing,
}

impl PauseReason {
    /// Its persisted wire name in `sink_state`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::ThreeDaysFailing => "three_days_failing",
        }
    }

    /// Parses a persisted wire name from `sink_state`.
    #[must_use]
    pub fn from_str_name(name: &str) -> Option<Self> {
        match name {
            "gone" => Some(Self::Gone),
            "three_days_failing" => Some(Self::ThreeDaysFailing),
            _ => None,
        }
    }
}

/// Which Sink Notification event was queued (ADRs 0026 and 0027).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkNotificationEvent {
    /// A Sink was paused after three days of total failure.
    Paused,
    /// A paused Sink recovered and delivered again.
    Recovered,
}

impl SinkNotificationEvent {
    /// Its wire event name in Foxhound's `notification_event` catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paused => "sink_paused",
            Self::Recovered => "sink_recovered",
        }
    }

    /// Parses a stored event name.
    #[must_use]
    pub fn from_str_name(name: &str) -> Option<Self> {
        match name {
            "sink_paused" => Some(Self::Paused),
            "sink_recovered" => Some(Self::Recovered),
            _ => None,
        }
    }
}

/// One queued Sink Notification (ADR 0026).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkNotification {
    /// The Sink's name.
    pub sink: String,
    /// The Run it belongs to.
    pub run_id: String,
    /// Which event occurred (`sink_paused` or `sink_recovered`).
    pub event: SinkNotificationEvent,
    /// Why it occurred (masked at `Scope::Kept` before persistence).
    pub reason: String,
    /// When it occurred, from the engine's `Clock`.
    pub occurred_at: SystemTime,
}

/// The delivery retry streak for one Sink, reset on every successful delivery.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetryState {
    /// When the current uninterrupted streak of failures began.
    pub first_failure_at: Option<SystemTime>,
    /// How many retryable attempts have failed in a row.
    pub consecutive_failures: usize,
}

impl RetryState {
    /// Clears the failure streak after a successful delivery.
    pub fn clear(&mut self) {
        self.first_failure_at = None;
        self.consecutive_failures = 0;
    }
}

/// What a retryable failure at `now` leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry after `wait`, at `next_at` on the engine's clock.
    Retry {
        /// Delay before the next attempt.
        wait: Duration,
        /// Wall-clock timestamp of the next attempt.
        next_at: SystemTime,
    },
    /// Pause the Sink after three days of total failure.
    Pause {
        /// Why it paused.
        reason: PauseReason,
    },
}

/// Pure function of the `Clock`'s reading (`now`): computes the next retry
/// state and decision after a transient failure, honouring `retry_after` and
/// pausing after [`PAUSE_AFTER`] (3 days) of total failure (ADR 0026).
#[must_use]
pub fn next_retry(
    state: &RetryState,
    now: SystemTime,
    retry_after: Option<Duration>,
) -> (RetryState, RetryDecision) {
    let first = state.first_failure_at.unwrap_or(now);
    let elapsed = now.duration_since(first).unwrap_or_default();
    let next_state = RetryState {
        first_failure_at: Some(first),
        consecutive_failures: state.consecutive_failures.saturating_add(1),
    };
    if elapsed >= PAUSE_AFTER {
        return (
            next_state,
            RetryDecision::Pause {
                reason: PauseReason::ThreeDaysFailing,
            },
        );
    }
    let base = SCHEDULE
        .get(state.consecutive_failures)
        .copied()
        .unwrap_or_else(|| SCHEDULE[SCHEDULE.len() - 1]);
    let wait = retry_after.map_or(base, |asked| base.max(asked));
    let next_at = now.checked_add(wait).unwrap_or(now);
    (next_state, RetryDecision::Retry { wait, next_at })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use proptest::prelude::*;

    use super::*;

    /// The Standard Webhooks schedule intervals match ADR 0026 verbatim and
    /// terminate in a three-day pause once 72 hours of total failure elapse.
    ///
    /// Covers: story:6-290
    #[test]
    fn standard_webhooks_schedule_pauses_after_three_days() {
        let mut state = RetryState::default();
        let mut now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut waits = Vec::new();
        loop {
            let (next_state, decision) = next_retry(&state, now, None);
            state = next_state;
            match decision {
                RetryDecision::Retry { wait, next_at } => {
                    assert_eq!(next_at, now + wait);
                    waits.push(wait);
                    now = next_at;
                }
                RetryDecision::Pause { reason } => {
                    assert_eq!(reason, PauseReason::ThreeDaysFailing);
                    break;
                }
            }
        }
        assert_eq!(waits, SCHEDULE.to_vec());
    }

    proptest! {
        /// For any clock reading, failure count and `Retry-After` wait, the
        /// schedule never retries sooner than the Standard Webhooks base
        /// interval or the `Retry-After` wait, and pauses once 72 hours of
        /// total failure have elapsed.
        ///
        /// Covers: story:6-290
        #[test]
        fn pure_schedule_honours_retry_after_and_three_day_bound(
            start_secs in 0u64..2_000_000_000u64,
            elapsed_secs in 0u64..(100 * 3600),
            failures in 0usize..20usize,
            retry_after_secs in prop::option::of(0u64..(48 * 3600)),
        ) {
            let start = UNIX_EPOCH + Duration::from_secs(start_secs);
            let now = start + Duration::from_secs(elapsed_secs);
            let state = RetryState {
                first_failure_at: Some(start),
                consecutive_failures: failures,
            };
            let ra = retry_after_secs.map(Duration::from_secs);
            let (next_state, decision) = next_retry(&state, now, ra);

            prop_assert_eq!(next_state.first_failure_at, Some(start));
            prop_assert_eq!(next_state.consecutive_failures, failures + 1);

            if elapsed_secs >= PAUSE_AFTER.as_secs() {
                prop_assert_eq!(
                    decision,
                    RetryDecision::Pause {
                        reason: PauseReason::ThreeDaysFailing,
                    }
                );
            } else {
                let base = SCHEDULE
                    .get(failures)
                    .copied()
                    .unwrap_or(SCHEDULE[SCHEDULE.len() - 1]);
                let expected_wait = ra.map_or(base, |asked| base.max(asked));
                prop_assert_eq!(
                    decision,
                    RetryDecision::Retry {
                        wait: expected_wait,
                        next_at: now + expected_wait,
                    }
                );
                if let Some(asked) = ra {
                    prop_assert!(expected_wait >= asked);
                }
                prop_assert!(expected_wait >= base);
            }
        }

        /// Any uninterrupted sequence of failures with arbitrary `Retry-After`
        /// values terminates in a 3-day pause in at most `SCHEDULE.len() + 1`
        /// attempts.
        ///
        /// Covers: story:6-290
        #[test]
        fn repeated_failures_always_terminate_in_pause(
            start_secs in 0u64..2_000_000_000u64,
            retry_afters in prop::collection::vec(prop::option::of(0u64..(36 * 3600)), 1..=12),
        ) {
            let mut state = RetryState::default();
            let mut now = UNIX_EPOCH + Duration::from_secs(start_secs);
            let mut paused = false;
            for step in 0..=SCHEDULE.len() {
                let ra = retry_afters
                    .get(step)
                    .copied()
                    .flatten()
                    .map(Duration::from_secs);
                let (next_state, decision) = next_retry(&state, now, ra);
                state = next_state;
                match decision {
                    RetryDecision::Retry { wait, next_at } => {
                        if let Some(asked) = ra {
                            prop_assert!(wait >= asked);
                        }
                        prop_assert_eq!(next_at, now + wait);
                        now = next_at;
                    }
                    RetryDecision::Pause { reason } => {
                        prop_assert_eq!(reason, PauseReason::ThreeDaysFailing);
                        paused = true;
                        break;
                    }
                }
            }
            prop_assert!(paused, "expected pause within {} steps", SCHEDULE.len() + 1);
        }

        /// Increasing `Retry-After` never shortens the computed wait.
        ///
        /// Covers: story:6-290
        #[test]
        fn larger_retry_after_never_shortens_wait(
            start_secs in 0u64..2_000_000_000u64,
            elapsed_secs in 0u64..(72 * 3600 - 1),
            failures in 0usize..9usize,
            ra_a in 0u64..(48 * 3600),
            ra_b in 0u64..(48 * 3600),
        ) {
            let (small, large) = if ra_a <= ra_b { (ra_a, ra_b) } else { (ra_b, ra_a) };
            let start = UNIX_EPOCH + Duration::from_secs(start_secs);
            let now = start + Duration::from_secs(elapsed_secs);
            let state = RetryState {
                first_failure_at: Some(start),
                consecutive_failures: failures,
            };
            let (_, dec_small) = next_retry(&state, now, Some(Duration::from_secs(small)));
            let (_, dec_large) = next_retry(&state, now, Some(Duration::from_secs(large)));
            match (dec_small, dec_large) {
                (
                    RetryDecision::Retry { wait: w_small, .. },
                    RetryDecision::Retry { wait: w_large, .. },
                ) => {
                    prop_assert!(w_large >= w_small);
                }
                _ => prop_assert!(false, "expected Retry before 72h"),
            }
        }
    }
}
