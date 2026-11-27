//! When a Schedule fires (ADR 0036): pure trigger parsing, DST-aware
//! Occurrence calculation, anchored fixed intervals, fixed jitter and
//! plain-words descriptions, with no clock of its own (ADR 0059).
//!
//! `translate` is `croner`'s translation point (ADR 0127), keeping
//! `croner`'s error text private so it reaches logs only.

use std::time::{Duration, SystemTime};

use croner::Cron;
use goodfetch_protocol::schedule::{
    CronTrigger, EveryTrigger, PreviewOccurrence, ScheduleTrigger, TriggerDescription,
    TriggerInvalidReason,
};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

mod describe;
mod translate;
mod zoned;

pub(crate) use translate::TriggerInvalid;
use zoned::ZonedInstant;

/// A validated trigger ready to yield Occurrences and a plain-words
/// description without re-parsing.
#[derive(Clone, Debug)]
pub(crate) enum ParsedTrigger {
    Cron {
        expression: String,
        cron: Box<Cron>,
    },
    Every {
        every_ms: u64,
        anchor: Option<SystemTime>,
    },
}

/// Checks `trigger`.
///
/// # Errors
///
/// [`TriggerInvalid`] when an `every` interval is 0 or a cron expression
/// fails to parse.
#[cfg(test)]
pub(crate) fn check(trigger: &ScheduleTrigger) -> Result<(), TriggerInvalid> {
    parse(trigger).map(|_| ())
}

/// Parses and validates `trigger`.
///
/// # Errors
///
/// [`TriggerInvalid`] when an `every` interval is 0 or a cron expression
/// fails to parse.
pub(crate) fn parse(trigger: &ScheduleTrigger) -> Result<ParsedTrigger, TriggerInvalid> {
    match trigger {
        ScheduleTrigger::Every(EveryTrigger { every: 0, .. }) => Err(TriggerInvalid::own(
            "0",
            0,
            TriggerInvalidReason::OutOfRange,
            "The trigger's `every` is 0 ms; give an interval of at least 1 ms.",
        )),
        ScheduleTrigger::Every(EveryTrigger { every, anchor }) => Ok(ParsedTrigger::Every {
            every_ms: *every,
            anchor: *anchor,
        }),
        ScheduleTrigger::Cron(CronTrigger { cron }) => {
            let parsed = translate::parse_cron(cron)?;
            Ok(ParsedTrigger::Cron {
                expression: cron.clone(),
                cron: Box::new(parsed),
            })
        }
    }
}

/// Draws a Schedule's fixed jitter offset once at creation, uniformly in
/// `0..upper_bound_ms` (`0` when `upper_bound_ms` is 0), from `draw`
/// (ADR 0059).
pub(crate) fn draw_jitter(upper_bound_ms: u64, mut draw: impl FnMut() -> u64) -> u64 {
    if upper_bound_ms <= 1 {
        return 0;
    }
    let threshold = u64::MAX - (u64::MAX % upper_bound_ms);
    loop {
        let sample = draw();
        if sample < threshold {
            return sample % upper_bound_ms;
        }
    }
}

/// The shortest interval between consecutive Occurrences of `trigger` in
/// milliseconds, used to enforce Operator policy's `schedule.min_interval`
/// cap (ADR 0036).
///
/// For `every`, returns `Some(every_ms)`. For `cron`, inspects consecutive
/// Occurrences in UTC over a bounded window and returns the minimum gap in
/// milliseconds (`None` if fewer than two Occurrences exist).
pub(crate) fn min_interval_ms(trigger: &ParsedTrigger) -> Option<u64> {
    const CRON_PROBE_COUNT: usize = 64;

    match trigger {
        ParsedTrigger::Every { every_ms, .. } => Some(*every_ms),
        ParsedTrigger::Cron { cron, .. } => {
            // Fixed UTC anchor so the interval check is independent of wall
            // clock and DST transitions (ADR 0059).
            let start =
                Timestamp::from_second(1_767_225_600).expect("invariant: 2026-01-01T00:00:00Z");
            let mut cursor = ZonedInstant(start.to_zoned(TimeZone::UTC));
            let mut previous: Option<Timestamp> = None;
            let mut shortest: Option<u64> = None;
            for _ in 0..CRON_PROBE_COUNT {
                let Ok(next) = cron.find_next_occurrence(&cursor, false) else {
                    break;
                };
                let next_ts = next.0.timestamp();
                if let Some(prev_ts) = previous {
                    let gap_ms = u64::try_from(prev_ts.duration_until(next_ts).as_millis().max(0))
                        .unwrap_or(u64::MAX);
                    shortest = Some(shortest.map_or(gap_ms, |best| best.min(gap_ms)));
                }
                previous = Some(next_ts);
                cursor = next;
            }
            shortest
        }
    }
}

/// The plain-words description of `trigger`: structured data for a Client's
/// own translation plus an English rendering (ADR 0073).
pub(crate) fn describe(trigger: &ParsedTrigger) -> TriggerDescription {
    match trigger {
        ParsedTrigger::Cron { expression, cron } => describe::describe_cron(expression, cron),
        ParsedTrigger::Every { every_ms, anchor } => describe::describe_every(*every_ms, *anchor),
    }
}

/// The next `count` Occurrences of `trigger` in `zone`, strictly after
/// `after`, with fixed `jitter` added to every Occurrence.
///
/// For an `every` trigger without an explicit anchor, `default_anchor` is
/// used (the Schedule's creation instant, or `after` in a preview).
///
/// # Errors
///
/// [`TriggerInvalid`] if `croner` cannot find a matching Occurrence (for
/// example, February 31st) or if an Occurrence overflows representable
/// timestamps.
pub(crate) fn occurrences(
    trigger: &ParsedTrigger,
    zone: &TimeZone,
    default_anchor: SystemTime,
    jitter: Duration,
    after: SystemTime,
    count: usize,
) -> Result<Vec<PreviewOccurrence>, TriggerInvalid> {
    if count == 0 {
        return Ok(Vec::new());
    }
    match trigger {
        ParsedTrigger::Cron { expression, cron } => {
            cron_occurrences(expression, cron, zone, jitter, after, count)
        }
        ParsedTrigger::Every { every_ms, anchor } => {
            let effective_anchor = anchor.unwrap_or(default_anchor);
            every_occurrences(*every_ms, effective_anchor, zone, jitter, after, count)
        }
    }
}

fn cron_occurrences(
    expression: &str,
    cron: &Cron,
    zone: &TimeZone,
    jitter: Duration,
    after: SystemTime,
    count: usize,
) -> Result<Vec<PreviewOccurrence>, TriggerInvalid> {
    let jitter_signed = SignedDuration::try_from(jitter).map_err(|_| {
        TriggerInvalid::own(
            expression,
            0,
            TriggerInvalidReason::OutOfRange,
            "The trigger's jitter exceeds the representable range.",
        )
    })?;
    let after_ts = Timestamp::try_from(after).map_err(|_| {
        TriggerInvalid::own(
            expression,
            0,
            TriggerInvalidReason::OutOfRange,
            "The starting instant is outside the representable range.",
        )
    })?;
    let search_ts = after_ts.checked_sub(jitter_signed).map_err(|_| {
        TriggerInvalid::own(
            expression,
            0,
            TriggerInvalidReason::OutOfRange,
            "The starting instant minus jitter is outside the representable range.",
        )
    })?;
    let mut cursor = ZonedInstant(search_ts.to_zoned(zone.clone()));
    let mut results = Vec::with_capacity(count);
    for _ in 0..count {
        let next = cron
            .find_next_occurrence(&cursor, false)
            .map_err(|error| translate::translate_cron_error(expression, error))?;
        let shifted = next.0.timestamp().checked_add(jitter_signed).map_err(|_| {
            TriggerInvalid::own(
                expression,
                0,
                TriggerInvalidReason::OutOfRange,
                "The next Occurrence is outside the representable range.",
            )
        })?;
        let zoned = shifted.to_zoned(zone.clone());
        results.push(PreviewOccurrence {
            at: SystemTime::from(shifted),
            local: zoned.to_string(),
        });
        cursor = next;
    }
    Ok(results)
}

fn every_occurrences(
    every_ms: u64,
    anchor: SystemTime,
    zone: &TimeZone,
    jitter: Duration,
    after: SystemTime,
    count: usize,
) -> Result<Vec<PreviewOccurrence>, TriggerInvalid> {
    let expr = every_ms.to_string();
    let anchor_ts = Timestamp::try_from(anchor).map_err(|_| {
        TriggerInvalid::own(
            &expr,
            0,
            TriggerInvalidReason::OutOfRange,
            "The anchor instant is outside the representable range.",
        )
    })?;
    let after_ts = Timestamp::try_from(after).map_err(|_| {
        TriggerInvalid::own(
            &expr,
            0,
            TriggerInvalidReason::OutOfRange,
            "The starting instant is outside the representable range.",
        )
    })?;
    let jitter_ns = i128::try_from(jitter.as_nanos()).map_err(|_| {
        TriggerInvalid::own(
            &expr,
            0,
            TriggerInvalidReason::OutOfRange,
            "The jitter exceeds the representable range.",
        )
    })?;
    let step_ns = i128::from(every_ms).checked_mul(1_000_000).ok_or_else(|| {
        TriggerInvalid::own(
            &expr,
            0,
            TriggerInvalidReason::OutOfRange,
            "The interval exceeds the representable range.",
        )
    })?;
    let base_ns = anchor_ts
        .as_nanosecond()
        .checked_add(jitter_ns)
        .ok_or_else(|| {
            TriggerInvalid::own(
                &expr,
                0,
                TriggerInvalidReason::OutOfRange,
                "The anchor plus jitter exceeds the representable range.",
            )
        })?;
    let cursor_ns = after_ts.as_nanosecond();
    let first_ns = if cursor_ns < base_ns {
        base_ns
    } else {
        let delta = cursor_ns - base_ns;
        let steps = (delta / step_ns).checked_add(1).ok_or_else(|| {
            TriggerInvalid::own(
                &expr,
                0,
                TriggerInvalidReason::OutOfRange,
                "The next Occurrence exceeds the representable range.",
            )
        })?;
        steps
            .checked_mul(step_ns)
            .and_then(|offset| base_ns.checked_add(offset))
            .ok_or_else(|| {
                TriggerInvalid::own(
                    &expr,
                    0,
                    TriggerInvalidReason::OutOfRange,
                    "The next Occurrence exceeds the representable range.",
                )
            })?
    };

    let mut results = Vec::with_capacity(count);
    for index in 0..count {
        let index_i128 = i128::try_from(index).expect("invariant: usize fits in i128");
        let occ_ns = index_i128
            .checked_mul(step_ns)
            .and_then(|offset| first_ns.checked_add(offset))
            .ok_or_else(|| {
                TriggerInvalid::own(
                    &expr,
                    0,
                    TriggerInvalidReason::OutOfRange,
                    "An Occurrence exceeds the representable range.",
                )
            })?;
        let ts = Timestamp::from_nanosecond(occ_ns).map_err(|_| {
            TriggerInvalid::own(
                &expr,
                0,
                TriggerInvalidReason::OutOfRange,
                "An Occurrence is outside the representable range.",
            )
        })?;
        let zoned = ts.to_zoned(zone.clone());
        results.push(PreviewOccurrence {
            at: SystemTime::from(ts),
            local: zoned.to_string(),
        });
    }
    Ok(results)
}

/// Fuzz entry point for `croner`'s input (ADR 0067), replayed from
/// `fuzz/cron_expression/` on every OS.
pub fn fuzz_cron_expression(data: &[u8]) {
    let Ok(expression) = std::str::from_utf8(data) else {
        return;
    };
    let trigger = ScheduleTrigger::Cron(CronTrigger {
        cron: expression.to_owned(),
    });
    match parse(&trigger) {
        Ok(parsed) => {
            let _ = describe(&parsed);
            let zone = TimeZone::UTC;
            let after = SystemTime::from(
                Timestamp::from_second(1_774_742_400).expect("invariant: valid fixed timestamp"),
            );
            let _ = occurrences(&parsed, &zone, after, Duration::ZERO, after, 3);
        }
        Err(refused) => {
            refused.assert_well_formed_for(expression);
        }
    }
}

#[cfg(test)]
mod tests;
