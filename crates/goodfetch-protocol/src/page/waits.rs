//! A page's app-state waits (ADRs 0080 and 0081): each waits for the state a
//! program's next step needs, under one deadline, its own `timeout` or else
//! `timeout.wait`, past which it fails with `deadline_exceeded` naming what
//! it waited for. Each is armed in the engine when its request arrives, in
//! the order of the page's calls, not when the program awaits its answer,
//! so the response a click triggers is caught by a waiter sent before the
//! click.
//!
//! `page.wait_for_url` passes on a committed navigation and on a change
//! within the document, a `pushState`, a `replaceState` or a hash change, so
//! a single-page app's routing is waited for like a navigation.
//! `networkidle` is reached once the document fired `load` and no request of
//! the page was in flight for 500 ms since, WebSocket and server-sent-event
//! connections left out, as are a service worker's own requests, which
//! belong to no page. A page has no sleep: a fixed pause waits for nothing
//! in particular. A page's events are awaited through `page.on`, which also
//! listens from when its call arrives.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::events::{PageExchange, PageRequest};
use super::functions::{Evaluated, World};
use super::route::UrlMatcher;
use super::{ARRIVED_IN, Locator, PageId};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};

/// A state the page's document reaches (Playwright's `LoadState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LoadState {
    /// The document fired `DOMContentLoaded`.
    DomContentLoaded,
    /// The document fired `load`.
    #[default]
    Load,
    /// The document fired `load`, and no request of the page was in flight
    /// for 500 ms since, WebSocket and server-sent-event connections left
    /// out.
    NetworkIdle,
}

/// What `page.wait_for_url` takes: the page, the URL it waits for, and the
/// state a new document must reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForUrlParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URL it waits for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: UrlMatcher,
    /// The state a new document at that URL must reach; `load` when
    /// absent. A change within the document waits for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wait_until: Option<LoadState>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `page.wait_for_request` and `page.wait_for_response` take: the
/// page, and the URLs of the requests they wait for; any request when none
/// is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForExchangeParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URLs it waits for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<UrlMatcher>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `page.wait_for_load_state` takes: the page and the state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForLoadStateParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The state; `load` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub state: Option<LoadState>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `page.wait_for_url`, `page.wait_for_load_state` and
/// `locator.wait_for` answer once the state they waited for came.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reached {}

/// What `page.wait_for_function` takes: the page, the function's source,
/// its argument, its world, and how often it is checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForFunctionParams {
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
    /// The milliseconds between two checks, at least 1; absent, about one
    /// frame's, 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub polling_interval: Option<u64>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `locator.wait_for_function` takes: the locator, the function's
/// source, its argument, its world, and how often it is checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocatorWaitForFunctionParams {
    /// The locator, whose one match the function takes first, resolved
    /// again at each check.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// The page function's JavaScript source: a function, called with the
    /// element and `arg`, or an expression, which takes neither.
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
    /// The milliseconds between two checks, at least 1; absent, about one
    /// frame's, 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub polling_interval: Option<u64>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// A state of the element a locator matches (Playwright's `waitFor`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ElementState {
    /// One element matches.
    Attached,
    /// No element matches.
    Detached,
    /// One element matches, with a box of its own and `visibility:
    /// visible`.
    #[default]
    Visible,
    /// No element matches, or the one that does is not visible.
    Hidden,
}

/// What `locator.wait_for` takes: the locator and the state of its element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocatorWaitForParams {
    /// The locator.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// The state; `visible` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub state: Option<ElementState>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// A count of matches, bounded on either side or both: an exact count is
/// both bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Count {
    /// The fewest; no bound when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub min: Option<u64>,
    /// The most; no bound when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max: Option<u64>,
}

/// What `locator.wait_for_count` takes: the locator and the count its
/// matches must reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForCountParams {
    /// The locator.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// The count.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: Count,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `locator.wait_for_count` answers: how many elements matched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Counted {
    /// The number of matches.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: u64,
}

/// `page.wait_for_url`.
pub const PAGE_WAIT_FOR_URL: Operation = Operation {
    name: "page.wait_for_url",
    description: "Waits, armed when the call arrives, until the page's URL matches: at once when it already does, else on a committed navigation, then until its document reaches `wait_until`, or on a change within the document, a `pushState`, a `replaceState` or a hash change. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<WaitForUrlParams>,
    result: schema_of::<Reached>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.wait_for_request`.
pub const PAGE_WAIT_FOR_REQUEST: Operation = Operation {
    name: "page.wait_for_request",
    description: "Waits, armed when the call arrives, until the page sends a request whose URL matches, and answers it, so a request an action triggers is caught by a waiter sent before the action. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<WaitForExchangeParams>,
    result: schema_of::<PageRequest>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.wait_for_response`.
pub const PAGE_WAIT_FOR_RESPONSE: Operation = Operation {
    name: "page.wait_for_response",
    description: "Waits, armed when the call arrives, until a request of the page whose URL matches gets its response, a redirect's included, and answers both, so a response an action triggers is caught by a waiter sent before the action, even one that arrives before the program awaits it. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<WaitForExchangeParams>,
    result: schema_of::<PageExchange>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.wait_for_load_state`.
pub const PAGE_WAIT_FOR_LOAD_STATE: Operation = Operation {
    name: "page.wait_for_load_state",
    description: "Waits until the page's document reached the load state: at once when it already did. `networkidle` is reached once the document fired `load` and no request of the page was in flight for 500 ms since, WebSocket and server-sent-event connections and a service worker's own requests left out. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<WaitForLoadStateParams>,
    result: schema_of::<Reached>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.wait_for_function`.
pub const PAGE_WAIT_FOR_FUNCTION: Operation = Operation {
    name: "page.wait_for_function",
    description: "Runs a page function in the page's main frame, as `page.evaluate` does, again at each polling interval and in each new document, until its result is truthy, and answers that result as JSON. A value outside JSON raises `page_value_invalid`, an exception thrown in the page `page_function_failed`. A main-world call is marked in the Session's Provenance. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Effect,
    kind: OperationKind::Waiter,
    reach: Reach::SdkOnly,
    params: schema_of::<WaitForFunctionParams>,
    result: schema_of::<Evaluated>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.wait_for_function`.
pub const LOCATOR_WAIT_FOR_FUNCTION: Operation = Operation {
    name: "locator.wait_for_function",
    description: "Waits, checking from when the call arrives, until a page function called with the one element the locator matches first and its one JSON argument second answers a truthy result, and answers that result as JSON, as `page.wait_for_function` does. The locator is resolved again at each check, so a re-render that replaces its element changes nothing, and a check while none or several elements match counts as falsy. A value outside JSON raises `page_value_invalid`, an exception thrown in the page `page_function_failed`, and a selector the page does not parse `invalid_params`. A main-world call is marked in the Session's Provenance. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Effect,
    kind: OperationKind::Waiter,
    reach: Reach::SdkOnly,
    params: schema_of::<LocatorWaitForFunctionParams>,
    result: schema_of::<Evaluated>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.wait_for`.
pub const LOCATOR_WAIT_FOR: Operation = Operation {
    name: "locator.wait_for",
    description: "Waits until the element the locator matches is in the state: attached or visible, one element; detached, none; hidden, none or one not visible. The locator is resolved again at each check, so a re-render that replaces its element changes nothing. Past its `timeout`, `timeout.wait` when the call gives none, several matches raise `target_not_unique` listing them, and anything else `deadline_exceeded` naming the state.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<LocatorWaitForParams>,
    result: schema_of::<Reached>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.wait_for_count`.
pub const LOCATOR_WAIT_FOR_COUNT: Operation = Operation {
    name: "locator.wait_for_count",
    description: "Waits until the number of elements the locator matches is within the count, and answers it. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<WaitForCountParams>,
    result: schema_of::<Counted>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
