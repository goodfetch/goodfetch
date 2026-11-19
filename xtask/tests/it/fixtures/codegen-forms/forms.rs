//! One entry of each kind an SDK gives a form of its own (ADR 0099), on the
//! handles a program holds: a round trip with a required text, a required
//! boolean and an optional parameter, a query, a waiter, a `new_` member
//! that opens a page, a value built without a call (a locator, whose ID is
//! its own value), a sub-object (`page.keyboard`), a `close` and a query
//! whose result is typed by the fields its call declares (`page.read`). The
//! `codegen` workspace takes this module when a test asks what each
//! language's generator writes for them. It declares `setting_invalid`,
//! which an SDK raises before sending a call's own `timeout` it refuses
//! (ADR 0080), the settings the calls' own `timeout` sets, whose defaults a
//! Python docstring states, and an event, `run.limited`, whose payload
//! reaches the types every error carries, which the client packages'
//! hand-written error base names (ADR 0079). `page.wait_for_ready` takes a
//! deprecated keyword, `poll`, and a choice with a deprecated value,
//! `loaded`, which each SDK gives an overload of its own (ADR 0099).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    Authority, Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass,
    Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting, Stability,
    ValueKind, schema_of,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Run, named by its program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunId(pub String);

/// A page of a Run, named by the engine when it opens it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageId(pub String);

/// What `run.start` starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStartParams {}

/// What `run.start` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStarted {
    /// The Run.
    pub run_id: RunId,
}

/// What `run.new_page` opens a page with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewPageParams {
    /// The Run.
    pub run_id: RunId,
    /// The page's viewport, the default one when left out.
    pub viewport: Option<String>,
}

/// What `run.new_page` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageOpened {
    /// The page.
    pub page_id: PageId,
}

/// What `page.goto` navigates with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GotoParams {
    /// The page.
    pub page_id: PageId,
    /// The URL to navigate to.
    pub url: String,
    /// The URL the navigation says it comes from.
    pub referrer: String,
    /// Whether the browser's cache is left out.
    pub fresh: bool,
    /// How long the navigation may take, in milliseconds.
    #[schemars(extend("setting" = "timeout.navigation"))]
    pub timeout: Option<u64>,
}

/// What `page.goto` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Navigated {
    /// The document's status.
    pub status: u16,
}

/// The page an operation with no other parameter names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    /// The page.
    pub page_id: PageId,
}

/// Whether a page is ready.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Readiness {
    /// Whether its document has loaded.
    pub ready: bool,
}

/// What `page.wait_for_ready` waits with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForReadyParams {
    /// The page.
    pub page_id: PageId,
    /// How long the wait may take, in milliseconds.
    #[schemars(extend("setting" = "timeout.wait"))]
    pub timeout: Option<u64>,
    /// A JSON value the wait checks against, of any type; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    /// How far the document must get; `interactive` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ReadyState>,
    /// How often the wait checks, in milliseconds: a deprecated keyword.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("deprecation" = {
        "since": "0.1",
        "replacement": null,
        "earliest_removal": "1.0",
        "note": "The engine picks how often a wait checks."
    }))]
    pub poll: Option<u64>,
}

/// What an operation that only acts answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Acted {}

/// What an action looks for on a page, resolved again at each use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    /// The page it looks on.
    pub page_id: PageId,
    /// A CSS selector.
    pub selector: String,
}

/// What `locator.click` clicks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClickParams {
    /// What to click.
    pub locator: Locator,
}

/// What `page.keyboard.press` presses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyboardPressParams {
    /// The page.
    pub page_id: PageId,
    /// The key to press.
    pub key: Key,
}

/// A key, as the page names it: one wire string of a type of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Key(pub String);

/// How far a page's document got: a choice of strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadyState {
    /// Parsed.
    Interactive,
    /// Loaded, with its subresources.
    Complete,
    /// Loaded, as an earlier version named `complete`: a deprecated value.
    #[schemars(extend("deprecation" = {
        "since": "0.1",
        "replacement": "complete",
        "earliest_removal": "1.0",
        "note": "The same state under its new name."
    }))]
    Loaded,
}

/// A field read from the page: the first element its selector finds, or
/// each one's, for a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PageField {
    /// The first element.
    Value(ValueField),
    /// Each element.
    List(ListField),
}

/// A value field: its element's text, read by its transform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValueField {
    /// A CSS selector.
    pub selector: String,
    /// Its transform: `text` by default.
    #[serde(default)]
    pub transform: Transform,
}

/// A list: its value field read from each element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListField {
    /// The field read from each element.
    pub list: ValueField,
}

/// A field's transform.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transform {
    /// The text read.
    #[default]
    Text,
    /// A number.
    Number,
}

/// What `page.read` reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadParams {
    /// The page.
    pub page_id: PageId,
    /// The fields, by name.
    pub fields: BTreeMap<String, PageField>,
}

/// What `page.read` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Read {
    /// Each field's value, by name.
    #[schemars(extend("typed_by" = "fields"))]
    pub values: Map<String, Value>,
    /// The names of the fields the page did not give.
    pub unread: Vec<String>,
}

const fn operation(
    name: &'static str,
    description: &'static str,
    kind: OperationKind,
    cancellable: bool,
) -> Operation {
    Operation {
        name,
        description,
        class: OperationClass::Read,
        kind,
        reach: Reach::AllSurfaces,
        params: schema_of::<PageParams>,
        result: schema_of::<Acted>,
        events: &[],
        cancellable,
        input: None,
        bound: None,
        stability: Stability::Stable,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }
}

/// The layers settings resolve through, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingsLayer {
    /// The built-in default.
    Default,
    /// The run.
    Run,
}

/// Where a run-layer value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunOrigin {
    /// A call's argument.
    Argument,
}

/// The setting that decided an outcome, which an error carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingLimit {
    /// The setting's key.
    pub key: String,
    /// Its value.
    pub value: Value,
    /// The layer that set it.
    pub layer: SettingsLayer,
    /// For the run layer, where the value came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<RunOrigin>,
}

/// The usage of the model calls a failed call made, which an error carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelUsage {
    /// The calls.
    pub calls: u64,
}

/// What `run.limited` reports: a setting that bounded the Run, and the
/// usage of its model calls so far, the types every error carries too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunLimited {
    /// The setting.
    pub limit: SettingLimit,
    /// The usage, once a model call was made.
    pub usage: Option<ModelUsage>,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        Operation {
            params: schema_of::<RunStartParams>,
            result: schema_of::<RunStarted>,
            ..operation("run.start", "Starts a Run.", OperationKind::Opener, false)
        },
        Operation {
            params: schema_of::<NewPageParams>,
            result: schema_of::<PageOpened>,
            ..operation(
                "run.new_page",
                "Opens a page in the Run.",
                OperationKind::RoundTrip,
                false,
            )
        },
        Operation {
            params: schema_of::<GotoParams>,
            result: schema_of::<Navigated>,
            ..operation(
                "page.goto",
                "Navigates the page to a URL.",
                OperationKind::RoundTrip,
                true,
            )
        },
        Operation {
            result: schema_of::<Readiness>,
            ..operation(
                "page.is_ready",
                "Whether the page's document has loaded.",
                OperationKind::Query,
                false,
            )
        },
        Operation {
            params: schema_of::<WaitForReadyParams>,
            ..operation(
                "page.wait_for_ready",
                "Waits until the page's document has loaded.",
                OperationKind::Waiter,
                true,
            )
        },
        Operation {
            params: schema_of::<ReadParams>,
            result: schema_of::<Read>,
            ..operation(
                "page.read",
                "Reads the named fields from the page.",
                OperationKind::Query,
                true,
            )
        },
        operation(
            "page.close",
            "Closes the page.",
            OperationKind::RoundTrip,
            false,
        ),
        Operation {
            params: schema_of::<KeyboardPressParams>,
            stability: Stability::Experimental,
            deprecated: Some(Deprecation {
                since: ARRIVED_IN,
                replacement: None,
                earliest_removal: ProtocolVersion { major: 1, minor: 0 },
                note: "A fixture entry, which no program should call.",
            }),
            ..operation(
                "page.keyboard.press",
                "Presses a key on the page.",
                OperationKind::RoundTrip,
                true,
            )
        },
        Operation {
            params: schema_of::<ClickParams>,
            ..operation(
                "locator.click",
                "Clicks what the locator finds.",
                OperationKind::RoundTrip,
                true,
            )
        },
    ],
    events: &[Event {
        name: "run.limited",
        description: "A setting bounded the Run.",
        class: EventClass::Lossless,
        payload: schema_of::<RunLimited>,
        stability: Stability::Stable,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }],
    handler_requests: &[],
    error_codes: &[ErrorCodeDefinition {
        name: "setting_invalid",
        description: "A value out of the setting's own range.",
        group: ErrorGroup::Program,
        retryable: false,
        exit_code: None,
        arrived_in: ARRIVED_IN,
    }],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        Setting {
            name: "timeout.navigation",
            description: "How long a navigation may take: 30 s.",
            kind: ValueKind::Duration,
            default: || serde_json::json!(30_000),
            clearable: false,
            authority: Authority::Every,
            weakens: None,
            fixed_for_run: false,
            unbuilt: None,
            arrived_in: ARRIVED_IN,
        },
        Setting {
            name: "timeout.wait",
            description: "How long a wait may take: 30 s.",
            kind: ValueKind::Duration,
            default: || serde_json::json!(30_000),
            clearable: false,
            authority: Authority::Every,
            weakens: None,
            fixed_for_run: false,
            unbuilt: None,
            arrived_in: ARRIVED_IN,
        },
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
