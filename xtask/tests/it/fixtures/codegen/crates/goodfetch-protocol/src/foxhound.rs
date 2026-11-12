//! Foxhound's own entries: what only Foxhound carries (ADRs 0095 and 0099).
//! Outside `crates/foxhound/`, only this crate and `xtask` name this module.
//! The entries here are samples until the crawl areas define the real ones.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Instrument, Metric, Operation,
    OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
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

/// What the operation `sample` reports.
const SAMPLE_CRAWLED: Event = Event {
    name: "sample.crawled",
    description: "The sample crawl finished.",
    class: EventClass::Telemetry,
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
    plugin_kinds: &[],
    metrics: &[Metric {
        name: "goodfetch.sample.crawled_pages",
        description: "Pages the sample crawl visited.",
        instrument: Instrument::Counter,
        unit: "{page}",
        labels: &[],
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
