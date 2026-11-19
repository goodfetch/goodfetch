//! A page's fields read in one call (ADR 0147): `page.read` takes named
//! fields, each declared as the code-free crawl declares one (ADR 0123),
//! and answers each one's value, or null with the reason the page did not
//! give it (ADR 0079). It is a query (ADR 0099): it reads the document as
//! it is, never waits and never escalates, an absent element being an
//! answer, so every fetch tier serves it, and in the browser it is one
//! engine read whatever the number of fields.
//!
//! A field is a value field, read from the first element its locator
//! finds; a list, `{"list": <value field>}`, read from every element its
//! locator finds; or rows, `{"rows": <locator>, "fields": {…}}`, a value
//! field of each name read under each element the base locator finds, the
//! first element under it each one's locator finds. A row lacking a field
//! keeps its other fields. A value field's locator may be the first rung of
//! a ladder, its `fallbacks` tried in turn when it finds nothing (issue
//! 1972).
//!
//! The declaration's types live here, on Collie's side of the protocol, and
//! Foxhound's code-free crawl takes the same in its `extraction.fields`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{ARRIVED_IN, PageId};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};
use crate::run::PointInTime;

/// One named field (ADR 0147): a value field, a list or rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PageField {
    /// The first element its locator finds.
    Value(ValueField),
    /// Every element its locator finds.
    List(ListField),
    /// Fields read under each element a base locator finds.
    Rows(RowsField),
}

/// A list: the value field read from every element its locator finds, in
/// document order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListField {
    /// The field read from each element.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub list: ValueField,
}

/// Rows: for each element the base locator finds, in document order, one
/// object of the named value fields, each read from the first element its
/// locator finds under that element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowsField {
    /// What finds each row's element.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rows: FieldLocator,
    /// Each row's fields, by name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: BTreeMap<String, ValueField>,
}

/// A value field (ADRs 0123 and 0147): the first element its locator
/// ladder finds in the page, its locator tried first, then each of its
/// `fallbacks` in turn until one finds an element; its text, its runs of
/// white space read as one space and trimmed, or one of its attributes, an
/// `href` or `src` resolved against the document's base URL; optionally a capture group of a regular
/// expression over that text; then its Value transform. A field the page
/// does not give is null, with why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValueField {
    /// What finds its element.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: FieldLocator,
    /// Its locator ladder's further rungs: when `locator` finds no
    /// element, the first of these, in order, that finds one supplies it;
    /// in a list, the first that finds any; in rows, the first that finds
    /// one under the row's element. Each is checked as `locator` is when
    /// the call or crawl starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fallbacks: Vec<FieldLocator>,
    /// The attribute read in place of the element's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attribute: Option<String>,
    /// The `match` that picks the text its transform reads.
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub matched: Option<FieldMatch>,
    /// Its Value transform: `text` by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub transform: FieldTransform,
}

impl ValueField {
    /// Its locator ladder's rungs, in the order they are tried: its
    /// `locator`, then each of its `fallbacks`.
    pub fn rungs(&self) -> impl Iterator<Item = &FieldLocator> {
        std::iter::once(&self.locator).chain(&self.fallbacks)
    }
}

/// What finds a field's element, named by `kind`: a CSS selector or an
/// `XPath` expression, each read alike on every tier, a page a fetch tier
/// serves from its document reading it as a browser reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldLocator {
    /// A CSS selector.
    Css {
        /// The selector.
        #[schemars(extend("arrived_in" = "0.1"))]
        selector: String,
    },
    /// An `XPath` 1.0 expression that finds nodes, with no namespace
    /// prefix and no variable; a value field reads the first node it finds
    /// in document order, when that node is an element, and a list every
    /// element it finds. Under a row's element it is evaluated with that
    /// element as its context node.
    Xpath {
        /// The expression.
        #[schemars(extend("arrived_in" = "0.1"))]
        selector: String,
    },
}

/// The `match` before a field's transform (ADR 0123): a capture group of a
/// regular expression, in the `regex` crate's syntax (ADR 0081), over the
/// text read; that group's text is what the transform reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldMatch {
    /// The expression's source.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: String,
    /// The capture group, 1 by default; 0 is the whole match.
    #[serde(default = "first_group")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group: u32,
}

const fn first_group() -> u32 {
    1
}

/// A field's Value transform (ADR 0123), named by `kind`, each
/// locale-dependent one stating its locale, a BCP 47 tag. A `number`,
/// `date` or `date_time` reads the machine-readable value of a `<time
/// datetime>`, a `<data value>` or an element's `content` attribute in its
/// invariant form (ISO 8601, a plain decimal with a dot), whatever the
/// locale; a text printed for people is read under its locale's CLDR
/// forms: a number in its digits, separators and grouping, a date or a
/// date and time in one of its short, medium or long forms with a
/// four-digit year; any other text is not a value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldTransform {
    /// The text read: a string.
    #[default]
    Text,
    /// An integer or decimal: a JSON number, never through a float, so a
    /// decimal a JSON number cannot hold exactly is not a value.
    Number {
        /// Its locale.
        #[schemars(extend("arrived_in" = "0.1"))]
        locale: String,
    },
    /// A date: `YYYY-MM-DD`.
    Date {
        /// Its locale.
        #[schemars(extend("arrived_in" = "0.1"))]
        locale: String,
    },
    /// A date and time: RFC 3339 with its offset, the one the text states,
    /// else `time_zone`'s at that time; one stating neither is not a value.
    DateTime {
        /// Its locale.
        #[schemars(extend("arrived_in" = "0.1"))]
        locale: String,
        /// The time zone, an IANA name, of a text that states none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        time_zone: Option<String>,
    },
}

/// A field its page did not give, so its value is null (ADRs 0079 and
/// 0123).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnreadField {
    /// The field, a JSON Pointer into the values read: `/price`, or
    /// `/offers/1/price` for a row's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// Why.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: UnreadReason,
    /// The text read, where one was: the text its `match` did not match, or
    /// that its transform does not read as a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: Option<String>,
}

/// Why a field is null.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnreadReason {
    /// Its locator found no element.
    NoElement,
    /// Its element has no such attribute.
    NoAttribute,
    /// Its `match` did not match the text, or its group took no part.
    NoMatch,
    /// Its transform does not read the text as a value.
    NotParsed,
    /// A `date_time` whose text states no offset, with no `time_zone`.
    NoTimeZone,
}

/// How a Value transform gave one field (ADR 0123).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TransformProvenance {
    /// The field, a JSON Pointer into the Record.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// The page text exactly as read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The transform.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub transform: ValueTransform,
    /// Its resolved locale, a BCP 47 tag, for a locale-dependent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locale: Option<String>,
    /// Its resolved time zone, an IANA name, for a date-time or a
    /// relative time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: Option<String>,
    /// The stated CLDR pattern, such as `dd/MM/yyyy`, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: Option<String>,
    /// A relative time's anchor: the fetch time of the page's document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub anchor: Option<PointInTime>,
}

/// One of the engine's closed list of Value transforms (ADR 0123).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueTransform {
    /// The element's text, or one attribute.
    Text,
    /// A capture group of a regular expression over that text.
    Match,
    /// A number or an amount under its locale.
    Number,
    /// A currency symbol or code under its locale.
    Currency,
    /// A date under its locale or a stated pattern.
    Date,
    /// A date and time with its time zone.
    DateTime,
    /// A relative time under its locale, anchored at the fetch time.
    RelativeTime,
    /// An exact text looked up in the transform's table.
    Map,
}

/// What `page.read` takes: the page and its named fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageReadParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The fields, by name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: BTreeMap<String, PageField>,
}

/// What `page.read` answers: each field's value by its name, and each
/// value the page did not give, with why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageRead {
    /// Each field's value, by name: a value field's value or null, a
    /// list's array of values or nulls, rows' array of objects, each
    /// holding every field of its row, null where its row lacks it. Its
    /// names and types follow `fields` (`typed_by`), from which each SDK
    /// types it (ADR 0147).
    #[schemars(extend("arrived_in" = "0.1", "typed_by" = "fields"))]
    pub values: Map<String, Value>,
    /// Each null the page did not give, in the order read; absent where
    /// every value was read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unread: Vec<UnreadField>,
}

/// `page.read`.
pub const PAGE_READ: Operation = Operation {
    name: "page.read",
    description: "Reads the named fields from the page's current document in one call and answers each one's value: a value field's from the first element its CSS or XPath locator finds, its text or one attribute, optionally a `match` group, then its Value transform; a list's from every element its locator finds; rows' from each element a base locator finds, each row one object of its fields read under that element. A value the page does not give is null, and `unread` names it with why: a row lacking a field keeps its other fields. It never waits: an absent element is an answer. It needs neither layout nor pixels, so every tier serves it, and in the browser it is one read of the page whatever the number of fields.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageReadParams>,
    result: schema_of::<PageRead>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

#[cfg(test)]
mod tests;
