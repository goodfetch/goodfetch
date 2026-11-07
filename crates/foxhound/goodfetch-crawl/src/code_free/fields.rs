//! A code-free crawl's named fields (spec #582; ADR 0123), as pure
//! functions of the first element each field's locator found: the text it
//! reads (the element's text, its runs of white space read as one space,
//! or an attribute, an `href` or `src` resolved against the document's
//! base URL), the `match` that picks a group of it, and the Value transform
//! that gives the field's value; what the Record's Provenance names of each
//! transform; the reason a field is null; the type each field takes in the
//! Dataset `pages`' schema; and the check that refuses at the start a field
//! the engine could never read.
//!
//! `number`, `date` and `date_time` read a machine-readable value (a
//! `<time datetime>`, a `<data value>` or an element's `content`
//! attribute, or one of those attributes named) in its invariant form,
//! whatever the locale, as ADR 0123 has them preferred. A text printed for
//! people is read under the field's locale's CLDR forms ([`printed`]): a
//! number in its digits, separators and grouping, a date or a date and
//! time in one of its short, medium or long forms; any other text is not a
//! value, and the field is null with `not_parsed`. A currency amount and a
//! relative time (`currency`, `relative_time`) are the Value transforms'
//! parsers' (#844, #848).

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_fetch_tier::{FieldElement, check_selector, check_xpath};
use goodfetch_frontier::Url;
use goodfetch_protocol::foxhound::crawl::{ExtractionField, FieldLocator, FieldTransform};
use goodfetch_protocol::foxhound::provenance::{
    TransformProvenance, UnreadField, UnreadReason, ValueTransform,
};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::fmt::temporal::Pieces;
use jiff::tz::TimeZone;
use regex::Regex;
use serde_json::{Number, Value, json};

mod printed;

/// The attributes whose value is machine-readable, read in its invariant
/// form: `<time datetime>`, `<data value>` and any element's `content`.
const MACHINE_READABLE: [(Option<&str>, &str); 3] = [
    (Some("time"), "datetime"),
    (Some("data"), "value"),
    (None, "content"),
];

/// The attributes holding a URL, resolved against the base URL.
const URL_ATTRIBUTES: [&str; 2] = ["href", "src"];

/// What one field read of its page: its value, null when the page did not
/// give it; each transform that gave it, as Provenance names it; and why
/// it is null.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Read {
    pub(crate) value: Value,
    pub(crate) transforms: Vec<TransformProvenance>,
    pub(crate) unread: Option<UnreadField>,
}

/// What a field reads with, built once per crawl as it starts
/// ([`Reader::of`]): its `match` pattern compiled, and its Value transform
/// with the forms its locale prints, so each page's text is only matched
/// against them.
pub(crate) struct Reader {
    /// Its `match` pattern and the group it picks.
    matched: Option<(Regex, usize)>,
    typed: Typed,
}

/// A field's Value transform, with what its locale prints learned: none
/// for a locale ICU4X has no forms for, whose printed texts are no value.
enum Typed {
    Text,
    Number {
        locale: String,
        printed: Option<printed::Numbers>,
    },
    Date {
        locale: String,
        printed: Option<printed::Dates>,
    },
    DateTime {
        locale: String,
        time_zone: Option<String>,
        printed: Option<printed::DateTimes>,
    },
}

impl Reader {
    /// What `field` reads with: a `match` pattern that does not compile,
    /// which [`check`] refused at the start, matches nothing.
    pub(crate) fn of(field: &ExtractionField, work: &CountedWork) -> Self {
        let forms = || work.count(Row::PageLocaleFormBuilds);
        let matched = field.matched.as_ref().and_then(|matched| {
            Some((
                work.regex(Row::PageSelectorCompilations, &matched.pattern)
                    .ok()?,
                usize::try_from(matched.group).ok()?,
            ))
        });
        let typed = match &field.transform {
            FieldTransform::Text => Typed::Text,
            FieldTransform::Number { locale } => {
                forms();
                Typed::Number {
                    locale: locale.clone(),
                    printed: printed::Numbers::of(locale),
                }
            }
            FieldTransform::Date { locale } => {
                forms();
                Typed::Date {
                    locale: locale.clone(),
                    printed: printed::Dates::of(locale),
                }
            }
            FieldTransform::DateTime { locale, time_zone } => {
                forms();
                Typed::DateTime {
                    locale: locale.clone(),
                    time_zone: time_zone.clone(),
                    printed: printed::DateTimes::of(locale),
                }
            }
        };
        Self { matched, typed }
    }
}

/// Where a field's text came from: an attribute in its invariant form, or
/// a text written for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Invariant,
    Printed,
}

/// Refuses the field `name` when the engine could never read it: a CSS
/// selector or an `XPath` expression it does not read on every tier, a
/// regular expression that does not compile or has no such group, a locale
/// that is no BCP 47 tag, or a time zone it has no rules for.
///
/// # Errors
///
/// What is wrong with it, in a sentence naming the field.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn check(name: &str, field: &ExtractionField) -> Result<(), String> {
    match &field.locator {
        FieldLocator::Css { selector } => {
            check_selector(selector).map_err(|invalid| invalid.to_string())
        }
        FieldLocator::Xpath { selector } => {
            check_xpath(selector).map_err(|invalid| invalid.to_string())
        }
    }
    .map_err(|invalid| format!("The field `{name}`'s {invalid}."))?;
    if let Some(matched) = &field.matched {
        let regex = Regex::new(&matched.pattern).map_err(|error| {
            format!("The field `{name}`'s `match` pattern does not compile: {error}")
        })?;
        let group = usize::try_from(matched.group).unwrap_or(usize::MAX);
        if group >= regex.captures_len() {
            return Err(format!(
                "The field `{name}`'s `match` pattern has no group {group}.",
                group = matched.group
            ));
        }
    }
    let (locale, time_zone) = match &field.transform {
        FieldTransform::Text => (None, None),
        FieldTransform::Number { locale } | FieldTransform::Date { locale } => (Some(locale), None),
        FieldTransform::DateTime { locale, time_zone } => (Some(locale), time_zone.as_ref()),
    };
    if let Some(locale) = locale.filter(|locale| !is_language_tag(locale)) {
        return Err(format!(
            "The field `{name}`'s locale `{locale}` is no BCP 47 language tag, such as `de-DE`."
        ));
    }
    if let Some(time_zone) = time_zone {
        TimeZone::get(time_zone).map_err(|_| {
            format!(
                "The field `{name}`'s time zone `{time_zone}` is no IANA time zone name the engine has rules for, such as `Europe/Berlin`."
            )
        })?;
    }
    Ok(())
}

/// Whether `tag` is written as a BCP 47 language tag: a language subtag
/// of 2, 3 or 5 to 8 letters, then subtags of 1 to 8 letters or digits.
fn is_language_tag(tag: &str) -> bool {
    let mut subtags = tag.split('-');
    let language_ok = subtags.next().is_some_and(|language| {
        matches!(language.len(), 2 | 3 | 5..=8)
            && language.bytes().all(|byte| byte.is_ascii_alphabetic())
    });
    language_ok
        && subtags.all(|subtag| {
            (1..=8).contains(&subtag.len())
                && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

/// The JSON Schema of a field: the type its transform gives, or null.
pub(crate) fn schema(field: &ExtractionField) -> Value {
    match field.transform {
        FieldTransform::Text => json!({ "type": ["string", "null"] }),
        FieldTransform::Number { .. } => json!({ "type": ["number", "null"] }),
        FieldTransform::Date { .. } => json!({ "type": ["string", "null"], "format": "date" }),
        FieldTransform::DateTime { .. } => {
            json!({ "type": ["string", "null"], "format": "date-time" })
        }
    }
}

/// The JSON Pointer of the top-level field `name` (RFC 6901).
fn pointer(name: &str) -> String {
    format!("/{}", name.replace('~', "~0").replace('/', "~1"))
}

/// What the field `name` reads of `element`, the first its locator found,
/// in a document whose base URL is `base_url`, with `reader`, the field's
/// own.
pub(crate) fn read(
    name: &str,
    (field, reader): (&ExtractionField, &Reader),
    element: Option<&FieldElement>,
    base_url: &str,
) -> Read {
    let field_pointer = pointer(name);
    let mut transforms = Vec::new();
    let unread =
        |reason: UnreadReason, text: Option<String>, transforms: Vec<TransformProvenance>| Read {
            value: Value::Null,
            transforms,
            unread: Some(UnreadField {
                field: field_pointer.clone(),
                reason,
                text,
            }),
        };
    let Some(element) = element else {
        return unread(UnreadReason::NoElement, None, transforms);
    };
    let Some((raw, form)) = text_of(field, element) else {
        return unread(UnreadReason::NoAttribute, None, transforms);
    };
    let provenance = |text: &str, transform: ValueTransform| TransformProvenance {
        field: pointer(name),
        text: text.to_owned(),
        transform,
        locale: None,
        time_zone: None,
        pattern: None,
        anchor: None,
    };
    let mut text = match (&field.attribute, form) {
        (Some(attribute), _) if URL_ATTRIBUTES.contains(&attribute.as_str()) => {
            resolved(&raw, base_url)
        }
        (None, Form::Printed) => collapsed(&raw),
        _ => raw.clone(),
    };
    let mut read_text = raw;
    if field.matched.is_some() {
        transforms.push(provenance(&text, ValueTransform::Match));
        let group = reader.matched.as_ref().and_then(|(regex, group)| {
            let captures = regex.captures(&text)?;
            Some(captures.get(*group)?.as_str().to_owned())
        });
        let Some(group) = group else {
            return unread(UnreadReason::NoMatch, Some(text), transforms);
        };
        text.clone_from(&group);
        read_text = group;
    }
    if field.transform == FieldTransform::Text {
        transforms.push(provenance(&read_text, ValueTransform::Text));
        return Read {
            value: Value::String(text),
            transforms,
            unread: None,
        };
    }
    match typed(&reader.typed, provenance(&text, ValueTransform::Text), form) {
        Ok((named, value)) => {
            transforms.push(named);
            Read {
                value,
                transforms,
                unread: None,
            }
        }
        Err(reason) => unread(reason, Some(text), transforms),
    }
}

/// The value a `number`, `date` or `date_time` gives of the text `named`
/// holds, read in `form`, and `named` completed as Provenance names that
/// transform: its locale, and the time zone that gave a date-time's
/// offset; why the field is null otherwise.
fn typed(
    transform: &Typed,
    mut named: TransformProvenance,
    form: Form,
) -> Result<(TransformProvenance, Value), UnreadReason> {
    let invariant = form == Form::Invariant;
    let value = match transform {
        Typed::Text => Some(Value::String(named.text.clone())),
        Typed::Number { locale, printed } => {
            named.transform = ValueTransform::Number;
            named.locale = Some(locale.clone());
            if invariant {
                number(&named.text)
            } else {
                printed
                    .as_ref()
                    .and_then(|printed| printed.read(&named.text))
                    .and_then(|read| number(&read))
            }
        }
        Typed::Date { locale, printed } => {
            named.transform = ValueTransform::Date;
            named.locale = Some(locale.clone());
            if invariant {
                date(&named.text)
            } else {
                printed
                    .as_ref()
                    .and_then(|printed| printed.read(&named.text))
                    .map(Value::String)
            }
        }
        Typed::DateTime {
            locale,
            time_zone,
            printed,
        } => {
            named.transform = ValueTransform::DateTime;
            named.locale = Some(locale.clone());
            let read = if invariant {
                date_time(&named.text, time_zone.as_deref())
            } else {
                printed
                    .as_ref()
                    .and_then(|printed| printed.read(&named.text))
                    .map_or(DateTime::NotParsed, |civil| {
                        zoned(civil, None, time_zone.as_deref())
                    })
            };
            match read {
                DateTime::Given { value, zoned } => {
                    if zoned {
                        named.time_zone.clone_from(time_zone);
                    }
                    Some(value)
                }
                DateTime::NoTimeZone => return Err(UnreadReason::NoTimeZone),
                DateTime::NotParsed => None,
            }
        }
    };
    value
        .map(|value| (named, value))
        .ok_or(UnreadReason::NotParsed)
}

/// The text `field` reads of `element`, and its form: the attribute it
/// names, invariant when machine-readable; else, for a `number`, `date` or
/// `date_time`, the element's machine-readable value when it has one; else
/// its text, printed. `None` when the element lacks the attribute named.
fn text_of(field: &ExtractionField, element: &FieldElement) -> Option<(String, Form)> {
    let attribute = |name: &str| {
        element
            .attributes
            .iter()
            .find(|(attribute, _)| attribute.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let machine_readable = |name: &str| {
        MACHINE_READABLE
            .iter()
            .any(|(tag, readable)| *readable == name && tag.is_none_or(|tag| tag == element.tag))
    };
    if let Some(name) = &field.attribute {
        let form = if machine_readable(name) {
            Form::Invariant
        } else {
            Form::Printed
        };
        return attribute(name).map(|value| (value, form));
    }
    if field.transform != FieldTransform::Text {
        let readable = MACHINE_READABLE.iter().find_map(|(tag, name)| {
            tag.is_none_or(|tag| tag == element.tag)
                .then(|| attribute(name))
                .flatten()
        });
        if let Some(value) = readable {
            return Some((value, Form::Invariant));
        }
    }
    Some((element.text.clone(), Form::Printed))
}

/// `text` with its runs of white space read as one space, and trimmed.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `url` resolved against `base_url`, as written when either does not
/// parse.
fn resolved(url: &str, base_url: &str) -> String {
    Url::parse(base_url)
        .and_then(|base| base.join(url.trim()))
        .map_or_else(|_| url.to_owned(), String::from)
}

/// The JSON number a plain decimal with a dot writes, exactly; `None` for
/// any other text, or one a JSON number cannot hold exactly (it never
/// passes through a float unchecked).
fn number(text: &str) -> Option<Value> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if whole.is_empty() || !all_digits(whole) || !all_digits(fraction) {
        return None;
    }
    if digits.contains('.') && fraction.is_empty() {
        return None;
    }
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    let sign = if negative && (whole != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    let canonical = if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    };
    if fraction.is_empty() {
        if let Ok(integer) = canonical.parse::<i64>() {
            return Some(Value::Number(integer.into()));
        }
        if let Ok(integer) = canonical.parse::<u64>() {
            return Some(Value::Number(integer.into()));
        }
    }
    let float: f64 = canonical.parse().ok()?;
    let written = Number::from_f64(float)?;
    (written.to_string() == canonical).then_some(Value::Number(written))
}

/// The date an ISO 8601 calendar date writes, `YYYY-MM-DD`; `None` for any
/// other text, or an impossible date.
fn date(text: &str) -> Option<Value> {
    let date: Date = text.parse().ok()?;
    (date.to_string() == text).then(|| Value::String(date.to_string()))
}

/// What a `date_time` read.
enum DateTime {
    /// Its RFC 3339 value, and whether `time_zone` gave its offset.
    Given { value: Value, zoned: bool },
    /// A date and time stating no offset, with no time zone to take one
    /// from.
    NoTimeZone,
    /// No ISO 8601 date and time.
    NotParsed,
}

/// The RFC 3339 date-time an ISO 8601 date and time writes, with the
/// offset it states, else `time_zone`'s at that time (a gap or overlap
/// resolved as Temporal's `compatible` does).
fn date_time(text: &str, time_zone: Option<&str>) -> DateTime {
    let Ok(pieces) = Pieces::parse(text) else {
        return DateTime::NotParsed;
    };
    let Some(time) = pieces.time() else {
        return DateTime::NotParsed;
    };
    if pieces.time_zone_annotation().is_some() {
        return DateTime::NotParsed;
    }
    let civil = pieces.date().to_datetime(time);
    zoned(civil, pieces.to_numeric_offset(), time_zone)
}

/// The RFC 3339 date-time of `civil`, with `offset` when its text states
/// one, else `time_zone`'s at that time (a gap or overlap resolved as
/// Temporal's `compatible` does).
fn zoned(
    civil: jiff::civil::DateTime,
    offset: Option<jiff::tz::Offset>,
    time_zone: Option<&str>,
) -> DateTime {
    let (zone, zoned) = match (offset, time_zone) {
        (Some(offset), _) => (TimeZone::fixed(offset), false),
        (None, Some(name)) => match TimeZone::get(name) {
            Ok(zone) => (zone, true),
            Err(_) => return DateTime::NotParsed,
        },
        (None, None) => return DateTime::NoTimeZone,
    };
    let Ok(at) = civil.to_zoned(zone) else {
        return DateTime::NotParsed;
    };
    let timestamp: Timestamp = at.timestamp();
    DateTime::Given {
        value: Value::String(timestamp.display_with_offset(at.offset()).to_string()),
        zoned,
    }
}

#[cfg(test)]
mod tests;
