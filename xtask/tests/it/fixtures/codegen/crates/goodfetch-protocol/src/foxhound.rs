//! Foxhound's own entries: what only Foxhound carries (ADRs 0095 and 0099).
//! Outside `crates/foxhound/`, only this crate and `xtask` name this module.
//! The entries here are samples until the crawl areas define the real ones.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Event, Instrument, Metric, Operation,
    OperationClass, ProtocolVersion, Stability, schema_of,
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
#[serde(deny_unknown_fields)]
pub struct SampleCrawlResult {
    /// How many pages were visited.
    pub visited: u32,
}

/// What `sample.crawled` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleCrawled {
    /// How many pages were visited.
    pub visited: u32,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    operations: &[Operation {
        name: "sample.crawl",
        defined_in: module_path!(),
        description: "Visits sample pages, each repeating a text.",
        class: OperationClass::Effect,
        params: schema_of::<SampleCrawlParams>,
        result: schema_of::<SampleCrawlResult>,
        events: &["sample.crawled"],
        cancellable: false,
        stability: Stability::Stable,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }],
    events: &[Event {
        name: "sample.crawled",
        defined_in: module_path!(),
        description: "The sample crawl finished.",
        payload: schema_of::<SampleCrawled>,
        stability: Stability::Stable,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }],
    error_codes: &[ErrorCodeDefinition {
        name: "sample_crawl_refused",
        defined_in: module_path!(),
        description: "The sample crawl refused its pages.",
        group: ErrorGroup::Refused,
        retryable: true,
        arrived_in: ARRIVED_IN,
    }],
    plugin_kinds: &[],
    metrics: &[Metric {
        name: "goodfetch.sample.crawled_pages",
        defined_in: module_path!(),
        description: "Pages the sample crawl visited.",
        instrument: Instrument::Counter,
        unit: "{page}",
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
};
