//! A page's named fields (ADRs 0123 and 0147), as pure functions of the
//! elements each field's locator found, whichever tier found them: the
//! text a value field reads of an element (its text, its runs of white
//! space read as one space, or an attribute, an `href` or `src` resolved
//! against the document's base URL), the `match` that picks a group of it,
//! and the Value transform that gives its value; a list's values, one per
//! element; rows' objects, one per row, of their own value fields; what
//! Provenance names of each transform; the reason a value is null; the type
//! each field takes in a Dataset's schema; and the check that refuses a
//! field the engine could never read, its locators aside, which the tier
//! that reads them checks. `page.read` and a code-free crawl's Records read
//! fields alike, through [`Fields`].
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

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::page::read::{
    FieldTransform, PageField, TransformProvenance, UnreadField, UnreadReason, ValueField,
    ValueTransform,
};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::fmt::temporal::Pieces;
use jiff::tz::TimeZone;
use regex::Regex;
use serde_json::{Map, Number, Value, json};
use url::Url;

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

/// One element a field's locator found in a page: its tag name,
/// lower-cased, its text content and its attributes, in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldElement {
    /// Its tag name, lower-cased.
    pub tag: String,
    /// Its text content.
    pub text: String,
    /// Its attributes, in document order.
    pub attributes: Vec<(String, String)>,
}

/// What a field's locators found in a page, as its kind reads them: a
/// value field's or a list's, one per rung of its locator ladder (issue
/// 1972), and rows' one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A value field's first element; none when its locator found none.
    One(Option<FieldElement>),
    /// Every element a list's locator found, in document order.
    List(Vec<FieldElement>),
    /// Each row, in document order: the first element under the row's
    /// element that each rung of each of its fields found, in the order of
    /// their names, then of the rungs.
    Rows(Vec<Vec<Option<FieldElement>>>),
}

/// What a page's fields read: each one's value by its name, null where the
/// page did not give it; each transform that gave a value, as Provenance
/// names it; and each null value with why, in the order read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    /// Each field's value, by name.
    pub values: Map<String, Value>,
    /// Each transform that gave a value.
    pub transforms: Vec<TransformProvenance>,
    /// Each value the page did not give, with why.
    pub unread: Vec<UnreadField>,
}

/// What a page's named fields read with, built once for all the pages
/// they are read of ([`Fields::of`]): each value field's reader, in the
/// order of the fields' names.
pub struct Fields {
    declared: Vec<(String, Declared)>,
}

/// One field, with what it reads with.
enum Declared {
    One(ValueField, Reader),
    List(ValueField, Reader),
    Rows(Vec<(String, ValueField, Reader)>),
}

impl Fields {
    /// What `fields` read with: each value field's `match` pattern
    /// compiled and its locale's forms learned, through `work`, the
    /// engine's Counted work. A field [`check`] refuses reads nothing.
    #[must_use]
    pub fn of(fields: &BTreeMap<String, PageField>, work: &CountedWork) -> Self {
        let declared = fields
            .iter()
            .map(|(name, field)| {
                let declared = match field {
                    PageField::Value(field) => {
                        Declared::One(field.clone(), Reader::of(field, work))
                    }
                    PageField::List(list) => {
                        Declared::List(list.list.clone(), Reader::of(&list.list, work))
                    }
                    PageField::Rows(rows) => Declared::Rows(
                        rows.fields
                            .iter()
                            .map(|(name, field)| {
                                (name.clone(), field.clone(), Reader::of(field, work))
                            })
                            .collect(),
                    ),
                };
                (name.clone(), declared)
            })
            .collect();
        Self { declared }
    }

    /// The values of the fields, `found` holding what each one's locator
    /// ladder found, in the order of their names, in a document whose base
    /// URL is `base_url`: one find per rung of a value field or a list, and
    /// rows' one find whose each row holds one entry per rung of each of
    /// its fields, in the order of their names. Each reads its first rung
    /// that found an element. A rung whose find answered another kind than
    /// its own, or none, found nothing.
    #[must_use]
    pub fn read(&self, found: Vec<Found>, base_url: &str) -> Values {
        let mut read = Values::default();
        let mut found = found.into_iter();
        for (name, declared) in &self.declared {
            let field_pointer = pointer(name);
            let value = match declared {
                Declared::One(field, reader) => {
                    let element = first_rung(&mut found, field, |found| match found {
                        Found::One(element) => element,
                        Found::List(_) | Found::Rows(_) => None,
                    });
                    read.take(read_one(
                        &field_pointer,
                        (field, reader),
                        element.as_ref(),
                        base_url,
                    ))
                }
                Declared::List(field, reader) => Value::Array(
                    first_rung(&mut found, field, |found| match found {
                        Found::List(elements) if !elements.is_empty() => Some(elements),
                        Found::One(_) | Found::List(_) | Found::Rows(_) => None,
                    })
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .map(|(index, element)| {
                        let at = format!("{field_pointer}/{index}");
                        read.take(read_one(&at, (field, reader), Some(element), base_url))
                    })
                    .collect(),
                ),
                Declared::Rows(fields) => match found.next() {
                    Some(Found::Rows(rows)) => Value::Array(
                        rows.iter()
                            .enumerate()
                            .map(|(index, row)| {
                                let mut entries = row.iter();
                                let mut object = Map::new();
                                for (name, field, reader) in fields {
                                    let element = first_rung(&mut entries, field, Option::as_ref);
                                    let at = format!("{field_pointer}/{index}{}", pointer(name));
                                    let one = read_one(&at, (field, reader), element, base_url);
                                    object.insert(name.clone(), read.take(one));
                                }
                                Value::Object(object)
                            })
                            .collect(),
                    ),
                    Some(Found::One(_) | Found::List(_)) | None => Value::Array(Vec::new()),
                },
            };
            read.values.insert(name.clone(), value);
        }
        read
    }
}

/// What the first of `field`'s rungs that `pick` takes found, of the next
/// finds of `found`, one per rung, every one of them taken from it.
fn first_rung<I: Iterator, T>(
    found: &mut I,
    field: &ValueField,
    mut pick: impl FnMut(I::Item) -> Option<T>,
) -> Option<T> {
    let mut first = None;
    for each in found.take(field.rungs().count()) {
        if first.is_none() {
            first = pick(each);
        }
    }
    first
}

impl Values {
    /// `one`'s value, its transforms and why it is null taken into these.
    fn take(&mut self, one: Read) -> Value {
        self.transforms.extend(one.transforms);
        self.unread.extend(one.unread);
        one.value
    }
}

/// What one value read of its page: its value, null when the page did not
/// give it; each transform that gave it, as Provenance names it; and why
/// it is null.
#[derive(Clone, Debug, PartialEq)]
struct Read {
    value: Value,
    transforms: Vec<TransformProvenance>,
    unread: Option<UnreadField>,
}

/// What a value field reads with, built once ([`Reader::of`]): its
/// `match` pattern compiled, and its Value transform with the forms its
/// locale prints, so each page's text is only matched against them.
struct Reader {
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
        printed: Option<Arc<printed::Numbers>>,
    },
    Date {
        locale: String,
        printed: Option<Arc<printed::Dates>>,
    },
    DateTime {
        locale: String,
        time_zone: Option<String>,
        printed: Option<Arc<printed::DateTimes>>,
    },
}

impl Reader {
    /// What `field` reads with: a `match` pattern that does not compile,
    /// which [`check`] refuses, matches nothing. Its locale's forms come
    /// from the engine's cache, learned once for the engine.
    fn of(field: &ValueField, work: &CountedWork) -> Self {
        let matched = field.matched.as_ref().and_then(|matched| {
            Some((
                work.regex(Row::PageSelectorCompilations, &matched.pattern)
                    .ok()?,
                usize::try_from(matched.group).ok()?,
            ))
        });
        let typed = match &field.transform {
            FieldTransform::Text => Typed::Text,
            FieldTransform::Number { locale } => Typed::Number {
                locale: locale.clone(),
                printed: work.built(Row::PageLocaleFormBuilds, locale, || {
                    printed::Numbers::of(locale)
                }),
            },
            FieldTransform::Date { locale } => Typed::Date {
                locale: locale.clone(),
                printed: work.built(Row::PageLocaleFormBuilds, locale, || {
                    printed::Dates::of(locale)
                }),
            },
            FieldTransform::DateTime { locale, time_zone } => Typed::DateTime {
                locale: locale.clone(),
                time_zone: time_zone.clone(),
                printed: work.built(Row::PageLocaleFormBuilds, locale, || {
                    printed::DateTimes::of(locale)
                }),
            },
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

/// Refuses the field `name`, or one of its rows' fields, when the engine
/// could never read it, its locators aside, which a tier checks as it finds
/// them: a regular expression that does not compile or has no such group,
/// a locale that is no BCP 47 tag, or a time zone it has no rules for. A
/// pattern compiles through `work`, the engine's Counted work.
///
/// # Errors
///
/// What is wrong with it, in a sentence naming the field: a row's field
/// as `rows/field`.
pub fn check(name: &str, field: &PageField, work: &CountedWork) -> Result<(), String> {
    match field {
        PageField::Value(field) => check_value(name, field, work),
        PageField::List(list) => check_value(name, &list.list, work),
        PageField::Rows(rows) => rows.fields.iter().try_for_each(|(field_name, field)| {
            check_value(&format!("{name}/{field_name}"), field, work)
        }),
    }
}

/// [`check`] of one value field.
fn check_value(name: &str, field: &ValueField, work: &CountedWork) -> Result<(), String> {
    if let Some(matched) = &field.matched {
        let regex = work
            .regex(Row::PageSelectorCompilations, &matched.pattern)
            .map_err(|error| {
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

/// The JSON Schema of a field's value: a value field's, the type its
/// transform gives, or null; a list's, an array of those; rows', an array
/// of objects holding each of their fields.
#[must_use]
pub fn schema(field: &PageField) -> Value {
    match field {
        PageField::Value(field) => value_schema(field),
        PageField::List(list) => json!({ "type": "array", "items": value_schema(&list.list) }),
        PageField::Rows(rows) => {
            let properties: Map<String, Value> = rows
                .fields
                .iter()
                .map(|(name, field)| (name.clone(), value_schema(field)))
                .collect();
            let required: Vec<&String> = rows.fields.keys().collect();
            json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
            })
        }
    }
}

/// The JSON Schema of a value field's value: the type its transform gives,
/// or null.
fn value_schema(field: &ValueField) -> Value {
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

/// What the value field at `field_pointer` reads of `element`, the one its
/// locator found, in a document whose base URL is `base_url`, with
/// `reader`, the field's own.
fn read_one(
    field_pointer: &str,
    (field, reader): (&ValueField, &Reader),
    element: Option<&FieldElement>,
    base_url: &str,
) -> Read {
    let mut transforms = Vec::new();
    let unread =
        |reason: UnreadReason, text: Option<String>, transforms: Vec<TransformProvenance>| Read {
            value: Value::Null,
            transforms,
            unread: Some(UnreadField {
                field: field_pointer.to_owned(),
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
        field: field_pointer.to_owned(),
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
fn text_of(field: &ValueField, element: &FieldElement) -> Option<(String, Form)> {
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
