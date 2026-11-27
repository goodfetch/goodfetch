//! Plain-words descriptions of a trigger (ADR 0073): a structured value a
//! Client can render in its own locale, plus an English rendering.

use std::time::SystemTime;

use croner::{Cron, JobType};
use goodfetch_protocol::schedule::{StructuredTriggerDescription, TriggerDescription};

use super::translate::{KNOWN_MACROS, field_spans};

/// Builds the plain-words description of a parsed cron trigger.
pub(super) fn describe_cron(expression: &str, cron: &Cron) -> TriggerDescription {
    let (seconds, minutes, hours, day_of_month, month, day_of_week) = normalized_fields(expression);
    TriggerDescription {
        structured: StructuredTriggerDescription::Cron {
            expression: expression.to_owned(),
            seconds,
            minutes,
            hours,
            day_of_month,
            month,
            day_of_week,
            fixed_time: cron.determine_job_type() == JobType::FixedTime,
        },
        english: cron.describe(),
    }
}

/// Builds the plain-words description of a fixed-interval trigger.
pub(super) fn describe_every(every_ms: u64, anchor: Option<SystemTime>) -> TriggerDescription {
    TriggerDescription {
        structured: StructuredTriggerDescription::Every {
            every: every_ms,
            anchor,
        },
        english: format!("Every {}.", human_duration(every_ms)),
    }
}

fn normalized_fields(expression: &str) -> (Option<String>, String, String, String, String, String) {
    let trimmed = expression.trim();
    if let Some((_, expanded)) = KNOWN_MACROS
        .iter()
        .find(|(name, _)| trimmed.eq_ignore_ascii_case(name))
    {
        let parts: Vec<&str> = expanded.split_whitespace().collect();
        return (
            None,
            parts[0].to_owned(),
            parts[1].to_owned(),
            parts[2].to_owned(),
            parts[3].to_owned(),
            parts[4].to_owned(),
        );
    }
    let spans = field_spans(expression);
    match spans.as_slice() {
        [(_, sec), (_, min), (_, hour), (_, dom), (_, mon), (_, dow)] => (
            Some((*sec).to_owned()),
            (*min).to_owned(),
            (*hour).to_owned(),
            (*dom).to_owned(),
            (*mon).to_owned(),
            (*dow).to_owned(),
        ),
        [(_, min), (_, hour), (_, dom), (_, mon), (_, dow)] => (
            None,
            (*min).to_owned(),
            (*hour).to_owned(),
            (*dom).to_owned(),
            (*mon).to_owned(),
            (*dow).to_owned(),
        ),
        _ => (
            None,
            "*".to_owned(),
            "*".to_owned(),
            "*".to_owned(),
            "*".to_owned(),
            "*".to_owned(),
        ),
    }
}

fn human_duration(ms: u64) -> String {
    const SECOND: u64 = 1_000;
    const MINUTE: u64 = 60 * SECOND;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    if ms.is_multiple_of(DAY) {
        plural(ms / DAY, "day", "days")
    } else if ms.is_multiple_of(HOUR) {
        plural(ms / HOUR, "hour", "hours")
    } else if ms.is_multiple_of(MINUTE) {
        plural(ms / MINUTE, "minute", "minutes")
    } else if ms.is_multiple_of(SECOND) {
        plural(ms / SECOND, "second", "seconds")
    } else {
        plural(ms, "millisecond", "milliseconds")
    }
}

fn plural(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        singular.to_owned()
    } else {
        format!("{count} {plural}")
    }
}
