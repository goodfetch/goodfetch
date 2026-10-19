//! Foxhound's own entries: what only Foxhound carries (ADRs 0095 and 0099).
//! Outside `crates/foxhound/`, only this crate and `xtask` name this module.
//! The entries here are samples until the crawl areas define the real ones.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Event, HookStage, Instrument, Metric, Operation,
    OperationClass, PluginKind, PluginMethod, ProtocolVersion, Setting, SharedType,
    SharedTypeValue, Stability, schema_of,
};
use crate::sample::SampleEchoParams;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `sample.crawl` starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleCrawlParams {
    /// The text each page repeats, a type Collie's catalog defines.
    pub echo: SampleEchoParams,
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
    payload: schema_of::<SampleCrawled>,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[Operation {
        name: "sample.crawl",
        description: "Visits sample pages, each repeating a text.",
        class: OperationClass::Effect,
        params: schema_of::<SampleCrawlParams>,
        result: schema_of::<SampleCrawlResult>,
        events: &[&SAMPLE_CRAWLED],
        cancellable: false,
        stability: Stability::Stable,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }],
    events: &[SAMPLE_CRAWLED],
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
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
    settings: &[Setting {
        name: "crawl.sample_pages",
        description: "How many pages the sample crawl visits.",
        arrived_in: ARRIVED_IN,
    }],
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
