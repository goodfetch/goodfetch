//! Page functions (ADRs 0019, 0021 and 0085): JavaScript source run in a
//! page with one explicit argument, values crossing as JSON both ways.
//! `page.evaluate` runs one in the page's main world, where the page's
//! globals are, or on request in an isolated world the page can neither
//! see nor hook, and answers its result as JSON; `locator.scroll_collect` reads a virtualized or infinite list whole
//! through one, scrolling it through the guard; `page.add_init_script`
//! registers one that runs in the main world of every document the page
//! loads afterwards, its frames' included, before the document's own
//! scripts. The engine serializes a result in the page, stopping at the
//! first value outside JSON, which raises `page_value_invalid` naming its
//! path and its kind, so no value is lost silently as Playwright's
//! `undefined` loses it; an exception thrown in the page raises
//! `page_function_failed`. A main-world call and an init script are marked
//! in the Session's Provenance, since each departs from `standard`'s
//! no-injection identity (ADR 0021).
//!
//! The source is a function if it parses as one, such as `(x) => x * 2` or
//! `function () { … }`, and is then called with the argument; otherwise it
//! is an expression, such as `document.title`, which takes no argument.
//! `undefined` follows `JSON.stringify`: dropped from objects, `null` in
//! arrays, and a result of `undefined` answers no value; `-0` arrives as
//! `0`.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ARRIVED_IN, Locator, PageId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, Reach, Stability,
    schema_of,
};
use crate::open_value::UnknownValue;

/// Where a page function runs (ADR 0085).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum World {
    /// The page's own world, where its globals are, as Playwright's and
    /// Puppeteer's default: the Session's Provenance marks the call.
    #[default]
    Main,
    /// A world of the engine's own in the page's document, which shares
    /// its DOM but neither sees nor is seen by the page's scripts.
    Isolated,
}

/// What `page.evaluate` takes: the page, the function's source, its
/// argument and its world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluateParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The page function's JavaScript source: a function, called with
    /// `arg`, or an expression, which takes none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub function: String,
    /// Its one argument, JSON; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub arg: Option<Value>,
    /// Where it runs; the main world when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub world: Option<World>,
}

/// What `page.evaluate` answers: the function's result, awaited when it is
/// a promise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Evaluated {
    /// The result as JSON; absent when it was `undefined`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Option<Value>,
}

/// What `locator.evaluate_all` takes: the locator, the function's source,
/// its argument and its world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocatorEvaluateAllParams {
    /// The locator, whose every match the function takes first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// The page function's JavaScript source: a function, called with the
    /// array of matches and `arg`, or an expression, which takes neither.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub function: String,
    /// Its one argument, JSON; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub arg: Option<Value>,
    /// Where it runs; the main world when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub world: Option<World>,
}

/// What `locator.scroll_collect` takes (#807): the locator, the page
/// function reading each match, its argument and world, and when the
/// collection ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocatorScrollCollectParams {
    /// The locator, whose every match the function reads, resolved again
    /// at each step.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// The page function's JavaScript source, called with each matched
    /// element and `arg`: it answers the row's `{ key, value }`, `key` a
    /// string, or `null` to skip the element.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub function: String,
    /// Its one argument, JSON; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub arg: Option<Value>,
    /// Where it runs; the isolated world when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub world: Option<World>,
    /// The rows at which the collection ends `max_rows`, at least 1;
    /// 10,000 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_rows: Option<u64>,
    /// The steps in a row without a new key at which it ends
    /// `end_of_list`, at least 1; 3 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub idle_steps: Option<u64>,
    /// How long a step waits for an element with a new key after its
    /// scroll, in milliseconds, at least 1; 2,000 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub step_wait: Option<u64>,
    /// How long the whole collection may take, in milliseconds, at least
    /// 1: absent for `timeout.wait`, `null` for none, though an enclosing
    /// timeout still applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `locator.scroll_collect` answers (#807): the rows, how the
/// collection ended and what the list declares, so a short read is never
/// silent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScrollCollected {
    /// Each new key's value, in the order its key was first seen.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rows: Vec<Value>,
    /// How it ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended: ListEnded,
    /// The rows the list declares through `aria-rowcount` or its items'
    /// `aria-setsize`; `null` when it declares none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub declared: Option<Declared>,
}

/// How a scroll collection ended, open (ADR 0079).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum ListEnded {
    /// `idle_steps` steps in a row brought no new key.
    EndOfList,
    /// It kept `max_rows` rows.
    MaxRows,
    /// How a newer engine says it ended, which this reader does not know,
    /// kept as received.
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

impl ListEnded {
    /// Its name on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(value) => value.as_str(),
            known => crate::wire_name::of(known),
        }
    }
}

/// The word of a list that declares its rows unknown, `aria-rowcount` or
/// `aria-setsize` of `-1`.
const UNKNOWN_ROWS: &str = "unknown";

/// The rows a list declares (WAI-ARIA's `aria-rowcount` and
/// `aria-setsize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declared {
    /// This many.
    Rows(u64),
    /// The list says it does not know, with `-1`.
    Unknown,
}

impl Serialize for Declared {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Rows(rows) => serializer.serialize_u64(*rows),
            Self::Unknown => serializer.serialize_str(UNKNOWN_ROWS),
        }
    }
}

impl<'de> Deserialize<'de> for Declared {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(word) if word == UNKNOWN_ROWS => Ok(Self::Unknown),
            Value::Number(number) => number.as_u64().map(Self::Rows).ok_or_else(|| {
                serde::de::Error::custom(format!("expected a row count, found `{number}`"))
            }),
            other => Err(serde::de::Error::custom(format!(
                "expected a row count or `{UNKNOWN_ROWS}`, found `{other}`"
            ))),
        }
    }
}

impl JsonSchema for Declared {
    fn schema_name() -> Cow<'static, str> {
        "Declared".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The rows a list declares: a count, or `unknown` for `-1`.",
            "oneOf": [
                {"type": "integer", "minimum": 0, "description": "This many."},
                {"const": UNKNOWN_ROWS, "description": "The list says it does not know."},
            ],
        })
    }
}

/// What `page.add_init_script` takes: the page, and the script's source and
/// argument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInitScriptParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The script's JavaScript source: a function, called with `arg`, or a
    /// script run as it is, which takes none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub function: String,
    /// The function's one argument, JSON; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub arg: Option<Value>,
}

/// What an init script's subscription answers when its page closes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InitScriptEnded {}

/// The kind of a value outside JSON (ADR 0085).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageValueKind {
    /// A DOM node, such as an element or a document.
    Node,
    /// A window, such as `window` itself.
    Window,
    /// `NaN`.
    NotANumber,
    /// `Infinity` or `-Infinity`.
    Infinity,
    /// A `BigInt`.
    Bigint,
    /// A `Symbol`.
    Symbol,
    /// A function.
    Function,
    /// An object reached again inside itself.
    Cycle,
    /// A `Map`.
    Map,
    /// A `Set`.
    Set,
    /// A `Date`.
    Date,
    /// A `RegExp`.
    Regexp,
    /// An `Error`.
    Error,
    /// Any other object whose prototype is not `Object`'s, such as a
    /// class's instance, a typed array or a promise inside a result.
    Object,
}

/// Which way a value outside JSON was going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageValueDirection {
    /// The page function's result, checked in the page.
    Result,
    /// Its argument, refused before it is sent.
    Argument,
}

/// What `page_value_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageValueInvalidDetails {
    /// Whether the value was the result or the argument.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub direction: PageValueDirection,
    /// Where the first value outside JSON is, from `$`, the whole value,
    /// such as `$.rows[2].cell`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Its kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: PageValueKind,
    /// Its constructor's name, when it has one, such as `HTMLDivElement`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub constructor: Option<String>,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A page function's result or argument holding a value outside JSON (ADR
/// 0085).
pub const PAGE_VALUE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_value_invalid",
    description: "A page function's result, or its argument, holds a value outside JSON, such as a DOM node, `NaN`, `Infinity`, a function, a `Map`, a `BigInt` or a cycle, which Playwright would have turned into `undefined`, silently. The details name the first such value's path from `$`, its kind and the fix: return the JSON the program needs, such as an element's text or attribute instead of the element.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `page_function_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageFunctionFailedDetails {
    /// The thrown error's name, such as `ReferenceError`; absent for a
    /// thrown value that is no error, and for an expression given an
    /// argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: Option<String>,
    /// What the page said, or why the engine refused the function.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// The page's stack, when it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stack: Option<String>,
    /// What to do instead, when the engine knows: a `ReferenceError`'s
    /// fix names the argument, which is all a page function sees of the
    /// program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: Option<String>,
}

/// A page function that threw in the page, or an expression given an
/// argument (ADR 0085).
pub const PAGE_FUNCTION_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_function_failed",
    description: "A page function threw in the page, or did not parse, or is an expression given an argument, which only a function takes. The details give the error's name, its message and the page's stack. A page function sees only the page and its one argument, never the program's variables, so a `ReferenceError`'s fix names the argument: pass the value in it.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `page.evaluate`.
pub const PAGE_EVALUATE: Operation = Operation {
    name: "page.evaluate",
    description: "Runs a page function in the page's main frame with its one JSON argument, in the main world, where the page's globals are, or in an isolated world the page neither sees nor hooks, and answers its result, awaited, as JSON; a result of `undefined` answers no value. A value outside JSON in the result raises `page_value_invalid` naming its path and kind, and an exception thrown in the page `page_function_failed`. It is no gesture: it runs beside the page's actions. A main-world call is marked in the Session's Provenance.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<EvaluateParams>,
    result: schema_of::<Evaluated>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.evaluate_all`.
pub const LOCATOR_EVALUATE_ALL: Operation = Operation {
    name: "locator.evaluate_all",
    description: "Runs a page function, as `page.evaluate` does, with the array of every element the locator matches first, none included, and its one JSON argument second, and answers its result, awaited, as JSON; the elements stay in the page, so a result holding one raises `page_value_invalid`. A selector the page does not parse raises `invalid_params`, and an exception thrown in the page `page_function_failed`. A main-world call is marked in the Session's Provenance.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<LocatorEvaluateAllParams>,
    result: schema_of::<Evaluated>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.scroll_collect` (#807).
pub const LOCATOR_SCROLL_COLLECT: Operation = Operation {
    name: "locator.scroll_collect",
    description: "Reads a virtualized or infinite list whole: at each step a page function, called with each element the locator matches and its one JSON argument, in the isolated world unless `world` says otherwise, answers the element's `{ key, value }` or `null` to skip it, and each new key's value is kept in first-seen order, a duplicate key keeping its first. Then the last match is scrolled into view, a gesture through the guard, and the step waits up to `step_wait` for an element with a new key. It ends `end_of_list` after `idle_steps` steps in a row without one, or `max_rows` once it kept that many rows, and answers the rows, how it ended and the rows the list declares through `aria-rowcount` or its items' `aria-setsize` (`unknown` for `-1`, `null` for none). Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded` whose `observed` gives the rows kept and the last key; a guard refusal stops it with the guard's code, and an exception thrown in the page raises `page_function_failed`. No other read scrolls.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<LocatorScrollCollectParams>,
    result: schema_of::<ScrollCollected>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.add_init_script`.
pub const PAGE_ADD_INIT_SCRIPT: Operation = Operation {
    name: "page.add_init_script",
    description: "Registers a script, registered when the call arrives, that runs in the main world of every document the page loads afterwards, its frames' included, before the document's own scripts: the call stays in flight, and a navigation sent after it waits until the script is registered. A function is called with its one JSON argument. The Session's Provenance marks the main-world injection. Cancelling the call, as closing its `Subscription` does, removes the script; it answers when the page closes.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<AddInitScriptParams>,
    result: schema_of::<InitScriptEnded>,
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
