//! Foxhound's own entries: what only Foxhound carries (ADRs 0095 and 0099).
//! Outside `crates/foxhound/`, only this crate and `xtask` name this module.
//! Its `sample` entries are samples until the crawl areas define the real
//! ones; `crawl` holds a crawl's start and the exchange that serves its work
//! items, `dataset` its Datasets and the Records pushed into them, `export` a
//! Dataset's Export, `frontier` what its Frontier answers each seed and enqueue, and
//! `outcomes` the reads of how its requests ended, `provenance` how each
//! Record was obtained, `report` what a crawl's
//! Run report adds, `progress` what its progress adds, and `session_pool` the Session of its Session pool that
//! serves each attempt, and `fetch` the tier that fetches a crawl
//! request's document, `sql_tab` the SQL tab's `dataset.query`, and `hook`
//! the Error codes a crawl's Hooks in code fail with, and `sink` a crawl's
//! Sinks' settings and Guarantee classes; `error_code` holds
//! every Foxhound Error code as a value of the shared `ErrorCode`.

pub mod block;
pub mod crawl;
pub mod dataset;
pub mod dataset_view;
pub mod error_code;
pub mod export;
pub mod fetch;
pub mod frontier;
pub mod hook;
pub mod outcomes;
pub mod progress;
pub mod provenance;
pub mod rejections;
pub mod report;
pub mod session_pool;
pub mod settings;
pub mod sink;
pub mod sql_tab;
pub mod usage;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, HookStage, Instrument, Metric,
    Operation, OperationClass, OperationKind, PluginKind, PluginMethod, ProtocolVersion, Reach,
    Setting, SharedType, SharedTypeValue, Stability, schema_of,
};
use crate::sample::SampleEchoed;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `sample.crawl` starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleCrawlParams {
    /// The text each page repeats, a type Collie's catalog defines. No
    /// request's params type, which holds `_meta` (ADR 0132), is another
    /// type's member.
    pub echo: SampleEchoed,
    /// How many pages to visit.
    pub pages: u32,
}

/// What `sample.crawl` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SampleCrawlResult {
    /// How many pages were visited.
    pub visited: u32,
}

/// What `sample.crawled` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SampleCrawled {
    /// How many pages were visited.
    pub visited: u32,
}

/// What a sample page filter is asked about a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SamplePageFilterAdmitParams {
    /// The page's URL.
    pub url: String,
}

/// What a sample page filter answers about a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SamplePageFilterAdmitResult {
    /// Whether the sample crawl visits the page.
    pub admitted: bool,
}

/// What the operation `sample` reports.
const SAMPLE_CRAWLED: Event = Event {
    name: "sample.crawled",
    description: "The sample crawl finished.",
    class: EventClass::Lossless,
    payload: schema_of::<SampleCrawled>,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.sample_pages`: the sample crawl's setting, which the sample
/// crawl does not read yet (the engine's list of unbuilt settings).
pub const CRAWL_SAMPLE_PAGES: Setting = Setting {
    name: "crawl.sample_pages",
    description: "How many pages the sample crawl visits.",
    kind: crate::catalog::ValueKind::Count,
    default: one,
    clearable: false,
    authority: crate::catalog::Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("sample_crawl_pages"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn one() -> serde_json::Value {
    serde_json::json!(1)
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[Operation {
        name: "sample.crawl",
        description: "Visits sample pages, each repeating a text.",
        class: OperationClass::Effect,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params: schema_of::<SampleCrawlParams>,
        result: schema_of::<SampleCrawlResult>,
        events: &[&SAMPLE_CRAWLED],
        cancellable: false,
        input: None,
        bound: None,
        stability: Stability::Stable,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }],
    events: &[SAMPLE_CRAWLED],
    handler_requests: &[],
    error_codes: &[ErrorCodeDefinition {
        name: "sample_crawl_refused",
        description: "The sample crawl refused its pages.",
        group: ErrorGroup::Refused,
        retryable: true,
        exit_code: None,
        arrived_in: ARRIVED_IN,
    }],
    plugin_kinds: &[PluginKind {
        name: "sample_page_filter",
        description: "Chooses the pages the sample crawl visits.",
        methods: &[PluginMethod {
            name: "admit",
            description: "Says whether the sample crawl visits a page.",
            params: schema_of::<SamplePageFilterAdmitParams>,
            result: schema_of::<SamplePageFilterAdmitResult>,
        }],
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
    metrics: &[Metric {
        name: "goodfetch.sample.crawled_pages",
        description: "Pages the sample crawl visited.",
        instrument: Instrument::Counter,
        unit: "{page}",
        labels: &[],
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
    settings: &[CRAWL_SAMPLE_PAGES],
    hook_stages: &[HookStage {
        name: "sample_crawled_page",
        description: "After the sample crawl visits a page.",
        arrived_in: ARRIVED_IN,
    }],
    shared_type_values: &[SharedTypeValue {
        shared_type: SharedType::UsageKind,
        name: "sample_crawled_pages",
        description: "Pages the sample crawl visited.",
        arrived_in: ARRIVED_IN,
    }],
};
