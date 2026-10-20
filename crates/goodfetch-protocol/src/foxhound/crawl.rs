//! A crawl's start and the exchange that serves its work items (ADRs 0003,
//! 0093 and 0102): `crawl.start` begins a crawl's Run from its seeds, and
//! the engine leases each work item to the program's handler as the handler
//! request `crawl_request.handle`, whose answer carries the Records the
//! attempt staged, committed once under its lease. Foxhound's own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, HandlerRequest, Operation, OperationClass,
    ProtocolVersion, Stability, schema_of,
};
use crate::run::{RUN_ID_PATTERN, RunId, RunStarted, Tags};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// One request a crawl starts from (ADR 0093): a URL, the Label that picks
/// its handler and its Payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its Label.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// The data the program attaches for its handler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<Value>,
}

/// A crawl's seeds, at least one (ADR 0102).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Seeds(Vec<Seed>);

impl Seeds {
    /// Each seed, in the order given.
    pub fn iter(&self) -> impl Iterator<Item = &Seed> {
        self.0.iter()
    }
}

impl<'de> Deserialize<'de> for Seeds {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let seeds = Vec::<Seed>::deserialize(deserializer)?;
        if seeds.is_empty() {
            return Err(serde::de::Error::custom(
                "a crawl starts from at least one seed",
            ));
        }
        Ok(Self(seeds))
    }
}

impl JsonSchema for Seeds {
    fn schema_name() -> Cow<'static, str> {
        "Seeds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let seed = generator.subschema_for::<Seed>();
        json_schema!({
            "description": "A crawl's seeds, at least one.",
            "type": "array",
            "items": seed,
            "minItems": 1,
        })
    }
}

/// What `crawl.start` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlStartParams {
    /// The Run's ID, as `run.start` takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1", "pattern" = RUN_ID_PATTERN))]
    pub run_id: Option<String>,
    /// The Run's opaque tags.
    #[serde(default, skip_serializing_if = "Tags::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tags: Tags,
    /// The requests the crawl starts from.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub seeds: Seeds,
}

/// One work item of a crawl as its handler receives it (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRequest {
    /// Its ID within the Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its Label.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// Its Payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<Value>,
}

/// What `crawl_request.handle` gives the handler: one attempt of one work
/// item, under its lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRequestHandleParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The work item.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request: CrawlRequest,
    /// Which attempt this is, from 1.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: u32,
    /// The lease's epoch, which only goes up (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
}

/// One Record a handler pushed into a Dataset (ADR 0103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushedRecord {
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The Record.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: Value,
}

/// What a handler answers once its attempt returned: the outputs it staged,
/// which commit once, under the attempt's lease (ADR 0003).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlRequestHandled {
    /// The Records it pushed, in order.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Vec<PushedRecord>,
}

/// `crawl.start`.
pub const CRAWL_START: Operation = Operation {
    name: "crawl.start",
    description: "Starts a crawl from its seeds, and answers once its Run is started. The engine then leases each work item to the program's handler as a `crawl_request.handle` request, and the crawl's Run finishes once every work item is done. A Run ID works as `run.start`'s.",
    class: OperationClass::IdempotentWrite,
    params: schema_of::<CrawlStartParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl_request.handle`.
pub const CRAWL_REQUEST_HANDLE: HandlerRequest = HandlerRequest {
    name: "crawl_request.handle",
    description: "Asks the program's handler to process one attempt of a work item under its lease. Its answer carries the Records the attempt staged, committed once while the lease holds; a failure, or no answer before the lease ends, commits nothing, and the work item is retried under the Run's policy.",
    params: schema_of::<CrawlRequestHandleParams>,
    result: schema_of::<CrawlRequestHandled>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A handler's own failure (ADRs 0093 and 0109).
pub const HANDLER_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "handler_failed",
    description: "The program's handler failed with its own error, such as an exception; the attempt committed nothing. It is retried only when `crawl.retry.handler_failed` is on, since a bug fails the same way every time.",
    group: ErrorGroup::Program,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An attempt whose lease ended with no word from the program (ADR 0093).
pub const HANDLER_LOST: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "handler_lost",
    description: "The attempt's lease ended with no answer from the program's handler: its process ended, its Connection dropped, or it hung. The attempt committed nothing and counts against `crawl.retry.max`.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CRAWL_START],
    events: &[],
    handler_requests: &[CRAWL_REQUEST_HANDLE],
    error_codes: &[HANDLER_FAILED, HANDLER_LOST],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
