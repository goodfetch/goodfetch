#![expect(
    clippy::doc_markdown,
    reason = "Covers: declarations carry raw Feature IDs (ADR 0072)"
)]

use std::time::{Duration, SystemTime};

use croner::errors::CronError;
use goodfetch_protocol::schedule::{
    CronTrigger, EveryTrigger, ScheduleTrigger, StructuredTriggerDescription, TriggerInvalidReason,
};
use goodfetch_test_support::time_zones;
use jiff::civil::Date;
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use rstest::rstest;

use super::translate::{translate_cron_error, translate_unrecognized};
use super::{check, describe, draw_jitter, occurrences, parse};

fn every(every: u64) -> ScheduleTrigger {
    ScheduleTrigger::Every(EveryTrigger {
        every,
        anchor: None,
    })
}

fn cron(expr: &str) -> ScheduleTrigger {
    ScheduleTrigger::Cron(CronTrigger {
        cron: expr.to_owned(),
    })
}

fn zoned(zone: &TimeZone, date: Date, hour: i8, minute: i8, second: i8) -> SystemTime {
    let dt = date.at(hour, minute, second, 0);
    let ts = zone
        .to_ambiguous_zoned(dt)
        .compatible()
        .expect("compatible timestamp")
        .timestamp();
    SystemTime::from(ts)
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

    assert_eq!(refused.details().reason, TriggerInvalidReason::OutOfRange);
    assert_eq!(refused.details().expression, "0");
}

/// Five fields, optional seconds, `L`, `W`, `#` and the `@daily`-style macros
/// all parse and produce structured plus English descriptions.
///
/// Covers: story:10-9, story:10-15
#[rstest]
#[case::five_fields("0 9 * * MON-FRI", true, None)]
#[case::six_fields_with_seconds("15 0 9 * * *", true, Some("15"))]
#[case::last_day_of_month("0 0 L * *", true, None)]
#[case::nearest_weekday("0 9 15W * *", true, None)]
#[case::nth_weekday("0 9 * * FRI#3", true, None)]
#[case::step_expression("*/15 * * * *", false, None)]
#[case::macro_daily("@daily", true, None)]
#[case::macro_hourly("@hourly", false, None)]
#[case::macro_weekly("@weekly", true, None)]
#[case::macro_monthly("@monthly", true, None)]
#[case::macro_yearly("@yearly", true, None)]
#[case::macro_annually("@annually", true, None)]
fn cron_forms_parse_and_describe(
    #[case] expression: &str,
    #[case] expected_fixed_time: bool,
    #[case] expected_seconds: Option<&str>,
) {
    let parsed = parse(&cron(expression)).expect("valid cron expression");
    let description = describe(&parsed);

    assert_ne!(description.english, "");
    match description.structured {
        StructuredTriggerDescription::Cron {
            expression: recorded,
            seconds,
            fixed_time,
            ..
        } => {
            assert_eq!(recorded, expression);
            assert_eq!(seconds.as_deref(), expected_seconds);
            assert_eq!(fixed_time, expected_fixed_time);
        }
        other @ StructuredTriggerDescription::Every { .. } => {
            panic!("expected Cron structured description, got {other:?}")
        }
    }
}

/// Every `croner::errors::CronError` variant plus an unknown party error
/// translates to a stable `TriggerInvalidReason`, and `croner`'s own text
/// never leaks into the Error's message, Debug output or wire details
/// (ADR 0127).
///
/// Covers: story:10-15, error:schedule_trigger_invalid
#[rstest]
#[case::empty_pattern(
    "",
    Some(CronError::EmptyPattern),
    TriggerInvalidReason::FieldCount,
    0,
    None
)]
#[case::invalid_date(
    "0 0 31 2 *",
    Some(CronError::InvalidDate),
    TriggerInvalidReason::OutOfRange,
    4,
    None
)]
#[case::invalid_time(
    "0 25 * * *",
    Some(CronError::InvalidTime),
    TriggerInvalidReason::OutOfRange,
    0,
    None
)]
#[case::time_search_limit_exceeded(
    "0 0 31 2 *",
    Some(CronError::TimeSearchLimitExceeded),
    TriggerInvalidReason::Other,
    0,
    None
)]
#[case::invalid_pattern_field_count(
    "0 9 * *",
    Some(CronError::InvalidPattern("CRONER_SENTINEL_FIELDS".to_owned())),
    TriggerInvalidReason::FieldCount,
    7,
    Some("CRONER_SENTINEL_FIELDS")
)]
#[case::invalid_pattern_too_many_fields(
    "0 0 9 * * * 2026",
    Some(CronError::InvalidPattern("CRONER_SENTINEL_YEARS".to_owned())),
    TriggerInvalidReason::FieldCount,
    12,
    Some("CRONER_SENTINEL_YEARS")
)]
#[case::invalid_pattern_bad_macro(
    "@bogus",
    Some(CronError::InvalidPattern("CRONER_SENTINEL_MACRO".to_owned())),
    TriggerInvalidReason::BadMacro,
    0,
    Some("CRONER_SENTINEL_MACRO")
)]
#[case::illegal_characters(
    "0 9 * * FOO",
    Some(CronError::IllegalCharacters("CRONER_SENTINEL_ILLEGAL".to_owned())),
    TriggerInvalidReason::UnknownField,
    8,
    Some("CRONER_SENTINEL_ILLEGAL")
)]
#[case::component_error_out_of_range(
    "0 25 * * *",
    Some(CronError::ComponentError("Number out of bounds. CRONER_SENTINEL_RANGE".to_owned())),
    TriggerInvalidReason::OutOfRange,
    2,
    Some("CRONER_SENTINEL_RANGE")
)]
#[case::component_error_bad_step(
    "*/0 * * * *",
    Some(CronError::ComponentError("Step cannot be zero. CRONER_SENTINEL_STEP".to_owned())),
    TriggerInvalidReason::BadStep,
    1,
    Some("CRONER_SENTINEL_STEP")
)]
#[case::component_error_unknown_syntax(
    "0 9 * * 1#bad",
    Some(CronError::ComponentError("Invalid nth specifier. CRONER_SENTINEL_NTH".to_owned())),
    TriggerInvalidReason::UnknownField,
    8,
    Some("CRONER_SENTINEL_NTH")
)]
#[case::unknown_party_error(
    "0 9 * * *",
    None,
    TriggerInvalidReason::Other,
    0,
    Some("CRONER_SENTINEL_FUTURE_VARIANT")
)]
fn croner_errors_translate_without_leaking_party_text(
    #[case] expression: &str,
    #[case] cron_error: Option<CronError>,
    #[case] expected_reason: TriggerInvalidReason,
    #[case] expected_position: u64,
    #[case] sentinel: Option<&str>,
) {
    let translated = match cron_error {
        Some(error) => translate_cron_error(expression, error),
        None => translate_unrecognized(expression, "CRONER_SENTINEL_FUTURE_VARIANT".to_owned()),
    };

    let details = translated.details();
    assert_eq!(details.expression, expression);
    assert_eq!(details.reason, expected_reason);
    assert_eq!(details.position, expected_position);

    if let Some(sentinel) = sentinel {
        let message = translated.to_string();
        let debug = format!("{translated:?}");
        let wire = serde_json::to_string(details).unwrap();
        assert!(
            !message.contains(sentinel),
            "party text leaked into message: {message}"
        );
        assert!(
            !debug.contains(sentinel),
            "party text leaked into Debug: {debug}"
        );
        assert!(
            !wire.contains(sentinel),
            "party text leaked into wire details: {wire}"
        );
    }
}

/// Real malformed expressions parsed end-to-end report the exact character
/// position where reading failed and the expected stable reason.
///
/// Covers: story:10-15, error:schedule_trigger_invalid
#[rstest]
#[case::empty("", TriggerInvalidReason::FieldCount, 0)]
#[case::too_few("0 9 * *", TriggerInvalidReason::FieldCount, 7)]
#[case::too_many("0 0 9 * * * 2026", TriggerInvalidReason::FieldCount, 12)]
#[case::bad_macro("@every_tuesday", TriggerInvalidReason::BadMacro, 0)]
#[case::macro_with_trailing("@daily extra", TriggerInvalidReason::BadMacro, 7)]
#[case::out_of_range_minute("60 9 * * *", TriggerInvalidReason::OutOfRange, 0)]
#[case::out_of_range_hour("0 24 * * *", TriggerInvalidReason::OutOfRange, 2)]
#[case::bad_step_zero("*/0 * * * *", TriggerInvalidReason::BadStep, 1)]
#[case::illegal_token("0 9 * * FOO", TriggerInvalidReason::UnknownField, 8)]
#[case::illegal_char_mid_field("0 9,1$ * * *", TriggerInvalidReason::UnknownField, 5)]
fn malformed_cron_expressions_pinpoint_position_and_reason(
    #[case] expression: &str,
    #[case] expected_reason: TriggerInvalidReason,
    #[case] expected_position: u64,
) {
    let refused = parse(&cron(expression)).unwrap_err();
    let details = refused.details();

    assert_eq!(details.expression, expression);
    assert_eq!(details.reason, expected_reason);
    assert_eq!(details.position, expected_position);
}

/// Finds the skipped local hour `(gap_hour, after_gap_hour)` on a zone's
/// `dst_gap` day, using `jiff` directly so the test reads real tzdb transitions.
fn gap_hours(zone: &TimeZone, day: Date) -> (i8, i8) {
    for hour in 0..24_i8 {
        if matches!(
            zone.to_ambiguous_zoned(day.at(hour, 30, 0, 0)).offset(),
            AmbiguousOffset::Gap { .. }
        ) {
            for after_hour in (hour + 1)..24_i8 {
                if matches!(
                    zone.to_ambiguous_zoned(day.at(after_hour, 0, 0, 0))
                        .offset(),
                    AmbiguousOffset::Unambiguous { .. }
                ) {
                    return (hour, after_hour);
                }
            }
        }
    }
    panic!("expected DST gap on {day} in {}", zone.iana_name().unwrap());
}

/// Finds the repeated local hour on a zone's `dst_overlap` day.
fn overlap_hour(zone: &TimeZone, day: Date) -> i8 {
    for hour in 0..24_i8 {
        if matches!(
            zone.to_ambiguous_zoned(day.at(hour, 30, 0, 0)).offset(),
            AmbiguousOffset::Fold { .. }
        ) {
            return hour;
        }
    }
    panic!(
        "expected DST overlap on {day} in {}",
        zone.iana_name().unwrap()
    );
}

fn dst_zones() -> Vec<(TimeZone, Date, Date)> {
    time_zones()
        .into_iter()
        .filter_map(|case| {
            let (gap, overlap) = (case.dst_gap?, case.dst_overlap?);
            let zone = TimeZone::get(&case.name).expect("shared table zone");
            Some((zone, gap, overlap))
        })
        .collect()
}

/// Every zone in test support's shared table yields strictly increasing
/// occurrences in that zone, independent of any host zone (ADR 0073).
///
/// Covers: story:10-15, story:10-17
#[test]
fn every_shared_table_zone_evaluates_triggers_in_its_own_zone() {
    let parsed_cron = parse(&cron("30 9 * * *")).unwrap();
    let parsed_every = parse(&every(3_600_000)).unwrap();
    let after = SystemTime::from(Timestamp::from_second(1_774_742_400).unwrap());

    for case in time_zones() {
        let zone = TimeZone::get(&case.name).expect("shared zone");
        let cron_occs = occurrences(&parsed_cron, &zone, after, Duration::ZERO, after, 4).unwrap();
        let every_occs =
            occurrences(&parsed_every, &zone, after, Duration::ZERO, after, 4).unwrap();

        assert_eq!(cron_occs.len(), 4, "{}", case.name);
        assert_eq!(every_occs.len(), 4, "{}", case.name);
        for occ in &cron_occs {
            assert!(occ.at > after);
            assert!(
                occ.local.contains(&case.name),
                "{} -> {}",
                case.name,
                occ.local
            );
            let z = Timestamp::try_from(occ.at).unwrap().to_zoned(zone.clone());
            assert_eq!((z.hour(), z.minute()), (9, 30), "{}", case.name);
        }
        for pair in every_occs.windows(2) {
            assert_eq!(
                pair[1].at.duration_since(pair[0].at).unwrap(),
                Duration::from_millis(3_600_000)
            );
        }
    }
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Fixed-time cron expressions scheduled inside a DST spring gap fire
    /// once at the first valid instant after the gap, and fixed-time
    /// expressions inside an autumn overlap fire once on the earlier pass
    /// (ADR 0036).
    ///
    /// Covers: story:10-17
    #[test]
    fn fixed_time_cron_fires_once_after_spring_gap_and_once_in_autumn_overlap(
        zone_index in 0usize..2,
        minute in 0u8..60,
    ) {
        let zones = dst_zones();
        let (zone, gap_day, overlap_day) = &zones[zone_index % zones.len()];
        let (gap_h, after_gap_h) = gap_hours(zone, *gap_day);
        let overlap_h = overlap_hour(zone, *overlap_day);

        // Spring gap: fixed time inside the skipped hour -> first valid time after gap.
        let expr_gap = format!("{minute} {gap_h} * * *");
        let parsed_gap = parse(&cron(&expr_gap)).unwrap();
        let before_gap = zoned(zone, *gap_day, 0, 0, 0);
        let spring = occurrences(&parsed_gap, zone, before_gap, Duration::ZERO, before_gap, 2).unwrap();
        let expected_after_gap = zoned(zone, *gap_day, after_gap_h, 0, 0);
        prop_assert_eq!(spring[0].at, expected_after_gap);
        let next_day = Timestamp::try_from(spring[1].at).unwrap().to_zoned(zone.clone());
        prop_assert_eq!(next_day.date(), gap_day.tomorrow().unwrap());
        prop_assert_eq!((next_day.hour(), next_day.minute()), (gap_h, i8::try_from(minute).unwrap()));

        // Autumn overlap: fixed time inside the repeated hour -> fires once only.
        let expr_overlap = format!("{minute} {overlap_h} * * *");
        let parsed_overlap = parse(&cron(&expr_overlap)).unwrap();
        let before_overlap = zoned(zone, *overlap_day, 0, 0, 0);
        let autumn = occurrences(&parsed_overlap, zone, before_overlap, Duration::ZERO, before_overlap, 2).unwrap();
        let first_z = Timestamp::try_from(autumn[0].at).unwrap().to_zoned(zone.clone());
        let second_z = Timestamp::try_from(autumn[1].at).unwrap().to_zoned(zone.clone());
        prop_assert_eq!(first_z.date(), *overlap_day);
        prop_assert_eq!((first_z.hour(), first_z.minute()), (overlap_h, i8::try_from(minute).unwrap()));
        // The second occurrence is the next day, not the second pass of the repeated hour.
        prop_assert_eq!(second_z.date(), overlap_day.tomorrow().unwrap());
    }

    /// Wildcard/step cron expressions skip a DST spring gap and fire at each
    /// matching wall-clock time on BOTH passes of an autumn overlap (ADR 0036).
    ///
    /// Covers: story:10-17
    #[test]
    fn step_cron_skips_spring_gap_and_fires_on_both_passes_of_autumn_overlap(
        zone_index in 0usize..2,
        step in prop_oneof![Just(10u8), Just(15u8), Just(20u8), Just(30u8)],
    ) {
        let zones = dst_zones();
        let (zone, gap_day, overlap_day) = &zones[zone_index % zones.len()];
        let (gap_h, after_gap_h) = gap_hours(zone, *gap_day);
        let overlap_h = overlap_hour(zone, *overlap_day);
        let per_hour = usize::from(60 / step);

        let expr = format!("*/{step} * * * *");
        let parsed = parse(&cron(&expr)).unwrap();

        // Spring gap: starting 1 minute before the gap skips straight to after_gap_h:00.
        let before_gap = zoned(zone, *gap_day, gap_h - 1, 59, 0);
        let spring = occurrences(&parsed, zone, before_gap, Duration::ZERO, before_gap, 2).unwrap();
        let first_spring = Timestamp::try_from(spring[0].at).unwrap().to_zoned(zone.clone());
        prop_assert_eq!((first_spring.hour(), first_spring.minute()), (after_gap_h, 0));
        prop_assert_eq!(
            spring[1].at.duration_since(spring[0].at).unwrap(),
            Duration::from_secs(u64::from(step) * 60)
        );

        // Autumn overlap: fires `per_hour` times in the first pass and `per_hour`
        // times in the second pass of `overlap_h`, every step elapsed minutes apart.
        let before_overlap = zoned(zone, *overlap_day, overlap_h - 1, 59, 0);
        let autumn = occurrences(
            &parsed,
            zone,
            before_overlap,
            Duration::ZERO,
            before_overlap,
            per_hour * 2,
        ).unwrap();
        let in_overlap_hour = autumn
            .iter()
            .filter(|occ| {
                let z = Timestamp::try_from(occ.at).unwrap().to_zoned(zone.clone());
                z.date() == *overlap_day && z.hour() == overlap_h
            })
            .count();
        prop_assert_eq!(in_overlap_hour, per_hour * 2);
        for pair in autumn.windows(2) {
            prop_assert_eq!(
                pair[1].at.duration_since(pair[0].at).unwrap(),
                Duration::from_secs(u64::from(step) * 60)
            );
        }
    }

    /// Fixed-interval triggers count elapsed time across both DST transitions,
    /// stay anchored to `anchor` regardless of `after`, and apply the exact
    /// same fixed jitter offset to every Occurrence (ADR 0036).
    ///
    /// Covers: story:10-16, story:10-17, story:10-18
    #[test]
    fn anchored_intervals_count_elapsed_time_and_apply_fixed_jitter(
        zone_index in 0usize..2,
        interval_secs in 60u64..7_200,
        anchor_shift_secs in 0u64..10_000,
        after_offset_secs in 0u64..20_000,
        jitter_bound_ms in 0u64..60_000,
        seed in any::<u64>(),
    ) {
        let zones = dst_zones();
        let (zone, gap_day, overlap_day) = &zones[zone_index % zones.len()];
        let every_ms = interval_secs * 1_000;
        let mut state = seed;
        let jitter_ms = draw_jitter(jitter_bound_ms, || {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            state
        });
        if jitter_bound_ms == 0 {
            prop_assert_eq!(jitter_ms, 0);
        } else {
            prop_assert!(jitter_ms < jitter_bound_ms);
        }
        let jitter = Duration::from_millis(jitter_ms);

        for day in [*gap_day, *overlap_day] {
            let base = zoned(zone, day, 0, 0, 0);
            let anchor = base - Duration::from_secs(anchor_shift_secs);
            let after = base + Duration::from_secs(after_offset_secs);
            let trigger = parse(&ScheduleTrigger::Every(EveryTrigger {
                every: every_ms,
                anchor: Some(anchor),
            })).unwrap();

            let occs = occurrences(&trigger, zone, after, jitter, after, 5).unwrap();
            prop_assert_eq!(occs.len(), 5);
            prop_assert!(occs[0].at > after);

            let anchor_ts = Timestamp::try_from(anchor).unwrap();
            let first_ts = Timestamp::try_from(occs[0].at).unwrap();
            let elapsed_ms = first_ts.duration_since(anchor_ts).as_millis() - i128::from(jitter_ms);
            prop_assert!(elapsed_ms >= 0);
            prop_assert_eq!(elapsed_ms % i128::from(every_ms), 0);

            // Consecutive occurrences are separated by exact elapsed `every_ms`.
            for pair in occs.windows(2) {
                prop_assert_eq!(
                    pair[1].at.duration_since(pair[0].at).unwrap(),
                    Duration::from_millis(every_ms)
                );
            }

            // Comparing against zero jitter shows the exact same fixed delay
            // at every occurrence.
            let unjittered = occurrences(
                &trigger,
                zone,
                after,
                Duration::ZERO,
                after - jitter,
                5,
            ).unwrap();
            for (with_j, without_j) in occs.iter().zip(unjittered.iter()) {
                let diff = Timestamp::try_from(with_j.at)
                    .unwrap()
                    .duration_since(Timestamp::try_from(without_j.at).unwrap());
                prop_assert_eq!(
                    diff,
                    SignedDuration::from_millis(i64::try_from(jitter_ms).unwrap())
                );
            }
        }
    }

    /// A cron trigger with a fixed jitter adds the exact same offset to every
    /// Occurrence, including when `after` falls between the unjittered match
    /// and the jittered firing time (ADR 0036).
    ///
    /// Covers: story:10-18
    #[test]
    fn cron_applies_the_same_fixed_jitter_to_every_occurrence(
        jitter_bound_ms in 1u64..300_000,
        seed in any::<u64>(),
    ) {
        let mut state = seed;
        let jitter_ms = draw_jitter(jitter_bound_ms, || {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            state
        });
        prop_assert!(jitter_ms < jitter_bound_ms);
        let jitter = Duration::from_millis(jitter_ms);

        let zone = TimeZone::get("Europe/Paris").unwrap();
        let parsed = parse(&cron("0 * * * *")).unwrap();
        // Start just past 09:00 UTC on a normal day.
        let unjittered_after = SystemTime::from(Timestamp::from_second(1_774_774_800).unwrap());
        let after = unjittered_after + Duration::from_millis(jitter_ms / 2);

        let unjittered = occurrences(&parsed, &zone, unjittered_after, Duration::ZERO, after - jitter, 4).unwrap();
        let jittered = occurrences(&parsed, &zone, after, jitter, after, 4).unwrap();

        prop_assert_eq!(jittered.len(), 4);
        for (with_j, without_j) in jittered.iter().zip(unjittered.iter()) {
            prop_assert!(with_j.at > after);
            prop_assert_eq!(with_j.at.duration_since(without_j.at).unwrap(), jitter);
        }
    }
}

mod cron_expression_parser {
    goodfetch_test_support::fuzz_replay!(crate::fuzz_cron_expression, "fuzz/cron_expression");
}
