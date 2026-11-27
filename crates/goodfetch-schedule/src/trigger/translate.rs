//! Translation point for `croner` (ADR 0127): parses cron expressions and
//! turns every `croner::errors::CronError` into [`TriggerInvalid`] with a
//! stable [`TriggerInvalidReason`] and character position. `croner`'s own
//! error text stays private to this module and reaches logs only; an
//! unrecognized failure becomes [`TriggerInvalidReason::Other`] logged at
//! `warn`.

use std::fmt;

use croner::Cron;
use croner::errors::CronError;
use croner::parser::{CronParser, FixedTimeRule, Seconds, Year};
use goodfetch_engine::CallError;
use goodfetch_protocol::schedule::{
    SCHEDULE_TRIGGER_INVALID, ScheduleTriggerInvalidDetails, TriggerInvalidReason,
};

/// The `@`-prefixed macros `croner` accepts, paired with their 5-field
/// expansions for structured descriptions and error positioning.
pub(super) const KNOWN_MACROS: [(&str, &str); 6] = [
    ("@yearly", "0 0 1 1 *"),
    ("@annually", "0 0 1 1 *"),
    ("@monthly", "0 0 1 * *"),
    ("@weekly", "0 0 * * SUN"),
    ("@daily", "0 0 * * *"),
    ("@hourly", "0 * * * *"),
];

/// Why a trigger is refused, as `schedule_trigger_invalid` carries it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TriggerInvalid {
    /// What the call answers.
    details: ScheduleTriggerInvalidDetails,
    /// What it says, in the engine's English (never carries `croner`'s text).
    message: String,
    /// `croner`'s own message, kept private so it reaches logs only (ADR 0127).
    party_message: Option<String>,
}

impl fmt::Debug for TriggerInvalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TriggerInvalid")
            .field("details", &self.details)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TriggerInvalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl TriggerInvalid {
    /// Builds a refusal produced by our own checks, with no third-party text.
    pub(super) fn own(
        expression: impl Into<String>,
        position: u64,
        reason: TriggerInvalidReason,
        message: impl Into<String>,
    ) -> Self {
        Self {
            details: ScheduleTriggerInvalidDetails {
                expression: expression.into(),
                position,
                reason,
            },
            message: message.into(),
            party_message: None,
        }
    }

    fn from_party(
        expression: &str,
        position: usize,
        reason: TriggerInvalidReason,
        message: impl Into<String>,
        party_message: Option<String>,
    ) -> Self {
        Self {
            details: ScheduleTriggerInvalidDetails {
                expression: expression.to_owned(),
                position: u64::try_from(position).unwrap_or(u64::MAX),
                reason,
            },
            message: message.into(),
            party_message,
        }
    }

    /// The structured refusal details carried on the wire.
    #[cfg(test)]
    pub(super) const fn details(&self) -> &ScheduleTriggerInvalidDetails {
        &self.details
    }

    /// Asserts that this refusal's stored expression and character position
    /// stay within `expression` (used by the fuzz target).
    pub(super) fn assert_well_formed_for(&self, expression: &str) {
        let char_len = u64::try_from(expression.chars().count()).unwrap_or(u64::MAX);
        assert!(
            self.details.position <= char_len,
            "position {} past expression char length {char_len} for {expression:?}",
            self.details.position
        );
        assert_eq!(self.details.expression, expression);
    }

    /// The `schedule_trigger_invalid` Error this refusal answers.
    pub(crate) fn into_call_error(self) -> CallError {
        CallError::new(&SCHEDULE_TRIGGER_INVALID, self.message, &self.details)
    }
}

impl From<TriggerInvalid> for CallError {
    /// The `schedule_trigger_invalid` Error the refusal answers.
    fn from(refused: TriggerInvalid) -> Self {
        refused.into_call_error()
    }
}

/// The `CronParser` used for every Schedule cron expression: five fields
/// plus optional seconds first, no year field, and ADR 0036's fixed-time
/// DST rule (`FixedTimeRule::NoWildcard`).
pub(super) fn cron_parser() -> CronParser {
    CronParser::builder()
        .seconds(Seconds::Optional)
        .year(Year::Disallowed)
        .fixed_time_rule(FixedTimeRule::NoWildcard)
        .build()
}

/// Parses `expression` with `croner`, translating any failure at this
/// boundary.
pub(super) fn parse_cron(expression: &str) -> Result<Cron, TriggerInvalid> {
    cron_parser()
        .parse(expression)
        .map_err(|error| translate_cron_error(expression, error))
}

/// Translates one [`CronError`] into [`TriggerInvalid`].
pub(super) fn translate_cron_error(expression: &str, error: CronError) -> TriggerInvalid {
    match error {
        CronError::EmptyPattern => TriggerInvalid::from_party(
            expression,
            0,
            TriggerInvalidReason::FieldCount,
            "The cron expression is empty; give five fields (minute hour day-of-month month day-of-week), optional seconds first, or a macro such as `@daily`.",
            None,
        ),
        CronError::InvalidDate => {
            let spans = field_spans(expression);
            let dom_index = if spans.len() == 6 { 3 } else { 2 };
            let position = spans.get(dom_index).map_or(0, |(offset, _)| *offset);
            TriggerInvalid::from_party(
                expression,
                position,
                TriggerInvalidReason::OutOfRange,
                "The cron expression names a calendar date that does not exist.",
                None,
            )
        }
        CronError::InvalidTime => TriggerInvalid::from_party(
            expression,
            0,
            TriggerInvalidReason::OutOfRange,
            "The cron expression names a time outside its valid range.",
            None,
        ),
        CronError::TimeSearchLimitExceeded => {
            tracing::warn!(
                expression,
                "croner search exceeded its time limit with no matching occurrence"
            );
            TriggerInvalid::from_party(
                expression,
                0,
                TriggerInvalidReason::Other,
                "The cron expression has no matching Occurrence within the search window.",
                None,
            )
        }
        CronError::InvalidPattern(party) => translate_invalid_pattern(expression, party),
        CronError::IllegalCharacters(party) => translate_illegal_characters(expression, party),
        CronError::ComponentError(party) => translate_component_error(expression, party),
    }
}

/// Translates an unrecognized third-party failure that no known variant
/// covers, logging it at `warn` (ADR 0127).
pub(super) fn translate_unrecognized(expression: &str, party_text: String) -> TriggerInvalid {
    tracing::warn!(
        expression,
        party_error = %party_text,
        "unrecognized croner error translated to `other`"
    );
    TriggerInvalid::from_party(
        expression,
        0,
        TriggerInvalidReason::Other,
        "The cron expression is not valid.",
        Some(party_text),
    )
}

fn translate_invalid_pattern(expression: &str, party: String) -> TriggerInvalid {
    let spans = field_spans(expression);
    if let Some((first_offset, first)) = spans.first().copied()
        && first.starts_with('@')
    {
        let position = if is_known_macro(first) && spans.len() > 1 {
            spans[1].0
        } else {
            first_offset
        };
        return TriggerInvalid::from_party(
            expression,
            position,
            TriggerInvalidReason::BadMacro,
            "The cron expression names an unknown or malformed macro; use `@yearly`, `@annually`, `@monthly`, `@weekly`, `@daily` or `@hourly`.",
            Some(party),
        );
    }
    if spans.len() < 5 {
        return TriggerInvalid::from_party(
            expression,
            expression.chars().count(),
            TriggerInvalidReason::FieldCount,
            "The cron expression has too few fields; give five fields (or six with seconds).",
            Some(party),
        );
    }
    if spans.len() > 6 {
        let position = spans[6].0;
        return TriggerInvalid::from_party(
            expression,
            position,
            TriggerInvalidReason::FieldCount,
            "The cron expression has too many fields; give five fields (or six with seconds).",
            Some(party),
        );
    }
    translate_unrecognized(expression, party)
}

fn translate_illegal_characters(expression: &str, party: String) -> TriggerInvalid {
    let spans = field_spans(expression);
    if let Some((offset, first)) = spans.first().copied()
        && first.starts_with('@')
    {
        return TriggerInvalid::from_party(
            expression,
            offset,
            TriggerInvalidReason::BadMacro,
            "The cron expression names an unknown macro; use `@yearly`, `@annually`, `@monthly`, `@weekly`, `@daily` or `@hourly`.",
            Some(party),
        );
    }
    let position = locate_failing_position(&spans, FailureHint::IllegalChar);
    TriggerInvalid::from_party(
        expression,
        position,
        TriggerInvalidReason::UnknownField,
        format!(
            "The cron expression has an unrecognized character or token at position {position}."
        ),
        Some(party),
    )
}

fn translate_component_error(expression: &str, party: String) -> TriggerInvalid {
    let spans = field_spans(expression);
    let reason = classify_component_error(&party);
    match reason {
        Some(TriggerInvalidReason::BadStep) => {
            let position = locate_failing_position(&spans, FailureHint::Step);
            TriggerInvalid::from_party(
                expression,
                position,
                TriggerInvalidReason::BadStep,
                format!("The cron expression has an invalid step at position {position}."),
                Some(party),
            )
        }
        Some(TriggerInvalidReason::OutOfRange) => {
            let position = locate_failing_position(&spans, FailureHint::Item);
            TriggerInvalid::from_party(
                expression,
                position,
                TriggerInvalidReason::OutOfRange,
                format!(
                    "The cron expression has a value out of range for its field at position {position}."
                ),
                Some(party),
            )
        }
        Some(TriggerInvalidReason::UnknownField) => {
            let position = locate_failing_position(&spans, FailureHint::Item);
            TriggerInvalid::from_party(
                expression,
                position,
                TriggerInvalidReason::UnknownField,
                format!(
                    "The cron expression has a malformed field specifier at position {position}."
                ),
                Some(party),
            )
        }
        _ => {
            let position = locate_failing_position(&spans, FailureHint::Item);
            tracing::warn!(
                expression,
                party_error = %party,
                "unrecognized croner component error translated to `other`"
            );
            TriggerInvalid::from_party(
                expression,
                position,
                TriggerInvalidReason::Other,
                "The cron expression is not valid.",
                Some(party),
            )
        }
    }
}

/// Maps `croner`'s `ComponentError` text inside this translation point only
/// (ADR 0127). Returns `None` for any wording we do not recognize so it
/// falls back to `Other` and logs at `warn`.
fn classify_component_error(party: &str) -> Option<TriggerInvalidReason> {
    if party.contains("Step cannot be zero")
        || party.contains("Invalid step")
        || party.contains("stepped range")
        || party.contains("after stepping")
    {
        Some(TriggerInvalidReason::BadStep)
    } else if party.contains("out of bounds")
        || party.contains("less than the input offset")
        || party.contains("Nth occurrence must be between")
    {
        Some(TriggerInvalidReason::OutOfRange)
    } else if party.contains("Invalid number")
        || party.contains("Invalid range")
        || party.contains("Invalid start of range")
        || party.contains("Invalid end of range")
        || party.contains("Invalid nth specifier")
        || party.contains("Invalid day for closest weekday")
        || party.contains("not supported by the current features")
    {
        Some(TriggerInvalidReason::UnknownField)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum FailureHint {
    IllegalChar,
    Step,
    Item,
}

/// Splits `expression` into `(char_offset, token)` spans for each
/// whitespace-delimited field.
pub(super) fn field_spans(expression: &str) -> Vec<(usize, &str)> {
    let mut spans = Vec::new();
    let mut start: Option<(usize, usize)> = None;
    for (char_index, (byte_index, ch)) in expression.char_indices().enumerate() {
        if ch.is_whitespace() {
            if let Some((start_char, start_byte)) = start.take() {
                spans.push((start_char, &expression[start_byte..byte_index]));
            }
        } else if start.is_none() {
            start = Some((char_index, byte_index));
        }
    }
    if let Some((start_char, start_byte)) = start {
        spans.push((start_char, &expression[start_byte..]));
    }
    spans
}

pub(super) fn is_known_macro(token: &str) -> bool {
    KNOWN_MACROS
        .iter()
        .any(|(name, _)| token.eq_ignore_ascii_case(name))
}

/// Probes each field independently with `croner` to pinpoint the exact
/// character offset of the first failing field and sub-item.
fn locate_failing_position(spans: &[(usize, &str)], hint: FailureHint) -> usize {
    let count = spans.len();
    if !(5..=6).contains(&count) {
        return 0;
    }
    let parser = cron_parser();
    for (index, &(field_offset, field)) in spans.iter().enumerate() {
        if !probe_field(parser, count, index, field) {
            return refine_field_position(parser, count, index, field_offset, field, hint);
        }
    }
    0
}

fn probe_field(parser: CronParser, field_count: usize, index: usize, value: &str) -> bool {
    let mut probe = vec!["*"; field_count];
    probe[index] = value;
    parser.parse(&probe.join(" ")).is_ok()
}

fn refine_field_position(
    parser: CronParser,
    field_count: usize,
    index: usize,
    field_offset: usize,
    field: &str,
    hint: FailureHint,
) -> usize {
    let mut item_char_offset = field_offset;
    for item in field.split(',') {
        if !probe_field(parser, field_count, index, item) {
            return match hint {
                FailureHint::Step => item
                    .char_indices()
                    .enumerate()
                    .find_map(|(pos, (_, ch))| (ch == '/').then_some(item_char_offset + pos))
                    .unwrap_or(item_char_offset),
                FailureHint::IllegalChar => {
                    first_illegal_char_offset(field_count, index, item_char_offset, item)
                }
                FailureHint::Item => item_char_offset,
            };
        }
        item_char_offset += item.chars().count() + 1;
    }
    field_offset
}

fn first_illegal_char_offset(
    field_count: usize,
    index: usize,
    item_offset: usize,
    item: &str,
) -> usize {
    // Normalize field index to the 6-field layout:
    // 0=sec, 1=min, 2=hour, 3=dom, 4=month, 5=dow.
    let canonical = if field_count == 5 { index + 1 } else { index };
    let upper = item.to_ascii_uppercase();
    let stripped = match canonical {
        4 => replace_valid_names(
            &upper,
            &[
                "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
            ],
        ),
        5 => replace_valid_names(&upper, &["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"]),
        _ => upper,
    };
    for (pos, ch) in stripped.chars().enumerate() {
        let base = matches!(ch, '*' | '/' | '0'..='9' | ',' | '-');
        let allowed = match canonical {
            3 => base || matches!(ch, 'L' | 'W' | '?'),
            5 => base || matches!(ch, '#' | 'L' | '?' | '+'),
            _ => base,
        };
        if !allowed {
            return item_offset + pos;
        }
    }
    item_offset
}

/// Replaces each recognized 3-letter name with three digits so character
/// positions of any remaining illegal characters stay exact.
fn replace_valid_names(text: &str, names: &[&str]) -> String {
    let mut out = text.to_owned();
    for name in names {
        out = out.replace(name, "000");
    }
    out
}
