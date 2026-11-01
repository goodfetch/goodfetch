//! What a handler's `enqueueLinks` reads and answers (ADRs 0093 and
//! 0120): `crawl_request.links` reads the elements a locator matches on the
//! attempt's page, as the program's Payload function reads them, and
//! `crawl_request.enqueue_links` adds their links, answering each element.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ARRIVED_IN, NotAddedReason};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};
use crate::page::Locator;
use crate::run::RunId;

/// What `crawl_request.links` takes: the locator whose elements an
/// attempt's `enqueueLinks` reads before its Payload function runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlRequestLinksParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The ID of the crawl request whose attempt reads the links.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
    /// The locator, on the page `crawl_request.handle` gave.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
}

/// One element an `enqueueLinks` locator matched, as its Payload function
/// reads it (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MatchedLink {
    /// Its `href` attribute, as written; none for an element without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub href: Option<String>,
    /// Its `href` resolved against the document's base URL, its query
    /// percent-encoded in the document's encoding: the URL it is fetched
    /// at; none when it has no `href` or the `href` does not parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// Its text content.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// Its attributes, by name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attributes: BTreeMap<String, String>,
}

/// What `crawl_request.links` answers: each element its locator matched,
/// in document order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRequestLinks {
    /// The elements.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub links: Vec<MatchedLink>,
}

/// What `crawl_request.enqueue_links` takes: every link a locator matches,
/// to add from one attempt under one Label (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlRequestEnqueueLinksParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The ID of the crawl request whose attempt enqueues.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
    /// The locator, on the page `crawl_request.handle` gave.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// Their Label.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// The one Payload every link takes; never with `payloads`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<Value>,
    /// Each matched element's own Payload, in the order
    /// `crawl_request.links` answered them, as the program's Payload
    /// function returned it: `null` skips that link, reported
    /// `skipped_by_payload`. Never with `payload`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payloads: Option<Vec<Value>>,
    /// URL patterns, globs as `crawl.scope.include`'s, of which a link must
    /// match one when there are any, besides the crawl's scope, which they
    /// only narrow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub include: Vec<String>,
    /// URL patterns a link must match none of, besides the crawl's scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exclude: Vec<String>,
    /// At most this many links are added; once they are, each later
    /// element whose link the Frontier would judge is not added, with
    /// `link_limit`. None for no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<u64>,
    /// Their priority among the requests of their domain, as
    /// `crawl_request.enqueue`'s; 0 by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub priority: i64,
}

/// What became of one element an `enqueueLinks` locator matched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnqueuedLink {
    /// The Frontier admitted its link as a crawl request.
    Added {
        /// The crawl request's ID within the Run.
        #[schemars(extend("arrived_in" = "0.1"))]
        id: String,
        /// Its URL as parsed and resolved, the URL its handler receives.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
    },
    /// The Frontier did not admit its link.
    NotAdded {
        /// Why.
        #[schemars(extend("arrived_in" = "0.1"))]
        reason: NotAddedReason,
        /// Its `href`, as written.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
    },
    /// The element has no `href`.
    NoLink,
    /// The program's Payload function skipped it, returning `null`.
    SkippedByPayload {
        /// Its `href`, as written.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
    },
}

/// What `crawl_request.enqueue_links` answers: what became of each element
/// its locator matched, in document order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnqueuedLinks {
    /// Each element's answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub links: Vec<EnqueuedLink>,
}

/// `crawl_request.links`.
pub const CRAWL_REQUEST_LINKS: Operation = Operation {
    name: "crawl_request.links",
    description: "Reads each element a locator matches on the page a handler's attempt was given, in document order, as a handler context's `enqueueLinks` gives them to its Payload function: its `href` as written, its URL (the `href` resolved against the document's base URL, its query percent-encoded in the document's encoding, as a browser encodes it), its text and its attributes. A page no fetch tier serves fails with `closed`, a selector that is no CSS selector with `invalid_params`, and an attempt whose lease no longer holds with `attempt_ended`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CrawlRequestLinksParams>,
    result: schema_of::<CrawlRequestLinks>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl_request.enqueue_links`.
pub const CRAWL_REQUEST_ENQUEUE_LINKS: Operation = Operation {
    name: "crawl_request.enqueue_links",
    description: "Adds every link a locator matches on the page a handler's attempt was given, a handler context's `enqueueLinks`, each a `GET` of its `href` resolved against the document's base URL (its `<base href>`, resolved against the URL the document was reached at after its redirects, or that URL itself), its query percent-encoded in the document's encoding as a browser encodes it, and answers each matched element in document order: added, with its crawl request's ID and URL; not added, with its reason, as `crawl_request.enqueue` answers; `no_link` for an element without an `href`; or `skipped_by_payload` for one whose entry in `payloads` is `null`. Every link takes `payload`, or its own entry of `payloads`, the program's Payload function's answers for the elements `crawl_request.links` read; a Payload that does not fit its Label fails the call with `payload_invalid` and adds none. `limit` adds at most that many links: once they are added, each later element whose link the Frontier would judge is not added, with `link_limit`. `include` and `exclude` only narrow the crawl's scope; a pattern the engine cannot match fails with `pattern_unsupported`. A page no fetch tier serves fails with `closed`, and an attempt whose lease no longer holds with `attempt_ended`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CrawlRequestEnqueueLinksParams>,
    result: schema_of::<EnqueuedLinks>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
