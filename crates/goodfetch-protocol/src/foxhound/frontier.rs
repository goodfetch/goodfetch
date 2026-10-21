//! What a crawl's Frontier answers (ADRs 0079, 0093, 0102 and 0120): every
//! seed and every enqueue is answered with an `Enqueued` value, added with
//! its ID or not added with its reason, so a crawl that does nothing says
//! why. `run.seeds` lists the answers each seed of a crawl got when its Run
//! began, and `crawl_request.enqueue` adds a request from inside a
//! handler's attempt, resolving a relative URL against the document's base
//! URL. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Why the Frontier did not add a request (ADRs 0093 and 0120).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NotAddedReason {
    /// The Frontier already admitted a request of the same identity.
    Duplicate,
    /// Its URL is outside the crawl's scope (`crawl.scope.*`).
    OutOfScope,
    /// The Run's domain policy refuses its host.
    DomainPolicy,
    /// It is deeper than `crawl.max_depth`.
    MaxDepth,
    /// The crawl already admitted `crawl.max_requests` requests.
    MaxRequests,
    /// Its URL's scheme is one the crawl cannot fetch: only `http` and
    /// `https` are.
    UnsupportedScheme,
    /// The WHATWG URL Standard's parser refuses its URL, or it is relative
    /// where nothing gives it a base, as a seed is.
    InvalidUrl,
    /// Its URL is longer than `crawl.max_url_length`.
    MaxUrlLength,
    /// Its URL's path has more segments than `crawl.max_path_segments`.
    MaxPathSegments,
    /// A segment of its URL's path repeats more than
    /// `crawl.max_segment_repeats`.
    MaxSegmentRepeats,
}

/// The Frontier's answer to a seed or an enqueue (ADRs 0079 and 0093): a
/// value, never an error, since a request is one among many.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Enqueued {
    /// The Frontier admitted it as a crawl request.
    Added {
        /// The crawl request's ID within the Run.
        #[schemars(extend("arrived_in" = "0.1"))]
        id: String,
        /// Its URL as the WHATWG URL Standard parsed it, resolved against
        /// its base: the URL its handler receives.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
    },
    /// The Frontier did not admit it.
    NotAdded {
        /// Why.
        #[schemars(extend("arrived_in" = "0.1"))]
        reason: NotAddedReason,
        /// Its URL as it was given.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
    },
}

/// What `run.seeds` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunSeeds {
    /// Each seed's answer, in the order the start gave the seeds; none for
    /// a Run that is no crawl.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub seeds: Vec<Enqueued>,
}

/// What `run.seeds` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunSeedsParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// The document a handler read, against which its relative URLs resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnqueueDocument {
    /// The URL the document was reached at, after every redirect.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The `href` of its first `<base>` element, as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub base_href: Option<String>,
}

/// What `crawl_request.enqueue` takes: one request to add, from one attempt
/// of a crawl request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlRequestEnqueueParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The ID of the crawl request whose attempt enqueues.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
    /// The URL to add, absolute or relative to the document's base URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its Label.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// The data the program attaches for its handler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<Value>,
    /// Its priority among the requests of its domain: higher goes first; 0
    /// by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub priority: i64,
    /// The document the handler read, whose base URL a relative `url`
    /// resolves against; absent, the crawl request's own URL is the base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub document: Option<EnqueueDocument>,
}

/// `run.seeds`.
pub const RUN_SEEDS: Operation = Operation {
    name: "run.seeds",
    description: "Lists each seed's answer, in the order the crawl's start gave them, as the Frontier answered it when the Run began: added, with its crawl request's ID and its URL as parsed, or not added, with its reason (`duplicate`, `domain_policy`, `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is `invalid_url`, since nothing gives it a base. So a crawl that does nothing says why. Read from the Run's committed state, so it works on a Run no program holds.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunSeedsParams>,
    result: schema_of::<RunSeeds>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl_request.enqueue`.
pub const CRAWL_REQUEST_ENQUEUE: Operation = Operation {
    name: "crawl_request.enqueue",
    description: "Adds one request to a crawl from inside a handler's attempt, a handler context's `enqueue`, and answers what the Frontier did with it: added, with its crawl request's ID and its URL as parsed, or not added, with its reason (`duplicate`, `unsupported_scheme`, `invalid_url` or a cap's name). A relative URL resolves against the document's base URL: its `<base href>`, resolved against the URL the document was reached at after its redirects, or that URL itself. The request sits one level deeper than the one whose attempt added it. An attempt whose lease no longer holds fails with `attempt_ended`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CrawlRequestEnqueueParams>,
    result: schema_of::<Enqueued>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_SEEDS, CRAWL_REQUEST_ENQUEUE],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
