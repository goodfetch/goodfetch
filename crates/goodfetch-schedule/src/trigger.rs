//! When a Schedule fires (ADR 0036): pure trigger parsing, DST-aware
//! Occurrence calculation, anchored fixed intervals, fixed jitter and
//! plain-words descriptions, with no clock of its own (ADR 0059).
//!
//! `translate` is `croner`'s translation point (ADR 0127), keeping
//! `croner`'s error text private so it reaches logs only.

use std::time::{Duration, SystemTime};

use croner::Cron;
use croner::time::Weekday;
use goodfetch_protocol::schedule::{
    CronTrigger, EveryTrigger, PreviewOccurrence, ScheduleTrigger, TriggerDescription,
    TriggerInvalidReason,
};
use jiff::civil::Date;
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
/// For `every`, returns `Some(every_ms)`. For `cron`, computes the exact
/// minimum gap in UTC across intra-day firing times and across the Gregorian
/// calendar cycle (two 28-year cycles, `2024..=2080`, so every month, leap
/// day and weekday combination repeats), returning `None` if fewer than two
/// Occurrences exist.
pub(crate) fn min_interval_ms(trigger: &ParsedTrigger) -> Option<u64> {
    match trigger {
        ParsedTrigger::Every { every_ms, .. } => Some(*every_ms),
        ParsedTrigger::Cron { cron, .. } => cron_min_interval_ms(cron),
    }
}

fn cron_min_interval_ms(cron: &Cron) -> Option<u64> {
    const SECONDS_PER_DAY: u64 = 86_400;
    const START_YEAR: i32 = 2024;
    // Two full 28-year leap cycles plus one wrap year so even rare AND-mode
    // patterns such as Feb 29 on a specific weekday match at least twice.
    const END_YEAR: i32 = 2080;

    let seconds = cron.pattern.seconds.values();
    let minutes = cron.pattern.minutes.values();
    let hours = cron.pattern.hours.values();
    let (&first_s, &last_s) = (seconds.first()?, seconds.last()?);
    let (&first_m, &last_m) = (minutes.first()?, minutes.last()?);
    let (&first_h, &last_h) = (hours.first()?, hours.last()?);

    let first_tod_s = u64::from(first_h) * 3_600 + u64::from(first_m) * 60 + u64::from(first_s);
    let last_tod_s = u64::from(last_h) * 3_600 + u64::from(last_m) * 60 + u64::from(last_s);

    let mut min_intra_day_s: Option<u64> = None;
    let mut record = |gap: u64| {
        min_intra_day_s = Some(min_intra_day_s.map_or(gap, |best| best.min(gap)));
    };
    for pair in seconds.windows(2) {
        record(u64::from(pair[1] - pair[0]));
    }
    for pair in minutes.windows(2) {
        let gap = u64::from(pair[1] - pair[0]) * 60 + u64::from(first_s) - u64::from(last_s);
        record(gap);
    }
    for pair in hours.windows(2) {
        let gap = u64::from(pair[1] - pair[0]) * 3_600
            + (u64::from(first_m) * 60 + u64::from(first_s))
            - (u64::from(last_m) * 60 + u64::from(last_s));
        record(gap);
    }

    let mut day_index: u64 = 0;
    let mut weekday = u32::try_from(Date::constant(2024, 1, 1).weekday().to_sunday_zero_offset())
        .expect("invariant: weekday offset is 0..=6");
    let mut matched_any_day = false;
    let mut prev_day: Option<u64> = None;
    let mut min_day_gap: Option<u64> = None;

    'years: for year in START_YEAR..=END_YEAR {
        for month in 1_u32..=12 {
            let month_len = days_in_month(year, month);
            if !cron.pattern.month_match(month).unwrap_or(false) {
                day_index += u64::from(month_len);
                weekday = (weekday + month_len) % 7;
                continue;
            }
            for day in 1..=month_len {
                let cron_weekday = Weekday::from_days_from_sunday(weekday);
                if cron
                    .pattern
                    .day_match(year, month, day, cron_weekday)
                    .unwrap_or(false)
                {
                    matched_any_day = true;
                    if let Some(prev) = prev_day {
                        let gap = day_index - prev;
                        min_day_gap = Some(min_day_gap.map_or(gap, |best| best.min(gap)));
                        if min_day_gap == Some(1) {
                            break 'years;
                        }
                    }
                    prev_day = Some(day_index);
                }
                day_index += 1;
                weekday = (weekday + 1) % 7;
            }
        }
    }

    if !matched_any_day {
        return None;
    }

    let min_inter_day_s = min_day_gap.map(|days| days * SECONDS_PER_DAY + first_tod_s - last_tod_s);
    let shortest_s = match (min_intra_day_s, min_inter_day_s) {
        (Some(intra), Some(inter)) => Some(intra.min(inter)),
        (Some(intra), None) => Some(intra),
        (None, Some(inter)) => Some(inter),
        (None, None) => None,
    }?;
    Some(shortest_s.saturating_mul(1_000))
}

const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
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
