//! What a crawl's Frontier answers (ADRs 0079, 0093, 0102 and 0120): every
//! seed and every enqueue is answered with an `Enqueued` value, added with
//! its ID or not added with its reason, so a crawl that does nothing says
//! why. `run.seeds` lists the answers each seed of a crawl got when its Run
//! began, and `crawl_request.enqueue` adds a request from inside a
//! handler's attempt, resolving a relative URL against the document's base
//! URL; `crawl_request.links` reads the elements a locator matches on
//! the attempt's page, as an `enqueueLinks` Payload function reads them,
//! and `crawl_request.enqueue_links` adds their links, each `href`'s query
//! percent-encoded in its document's encoding, answering each element
//! added, not added with its reason, `no_link` or `skipped_by_payload`.
//! A crawl request is sent with a method and a body, which with its
//! URL's key make its identity, unless the program declares its `key`.
//! `run.add_seed` adds a seed to a running crawl, and `run.add_seeds` takes
//! a stream of them under credits, answering each in order. The
//! `crawl.scope.*` settings say which links a crawl may follow, fixed for
//! the Run's life; `crawl.max_depth` and `crawl.max_requests` bound it, and
//! three caps on a URL's shape end the traps no rule learned;
//! `crawl.path_budgets` bounds sections of a site. A Payload is refused at
//! its enqueue with `payload_invalid` when it does not fit its Label's JSON
//! Schema or is longer than `crawl.payload.max_size`, and a resume whose
//! Label's Payload schema changed under pending Payloads that fail it with
//! `payload_schema_changed` (ADR 0082). Foxhound's own.
//!
//! Size: one catalog module, the contract of a crawl's Frontier: what it
//! answers and the settings that bound it, each a flat declaration.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, SharedType, SharedTypeValue, Stability, StreamInput,
    ValueKind, ValueShape, schema_of,
};
use crate::foxhound::crawl::Seed;
use crate::jsonrpc::RequestId;
use crate::open_value::UnknownValue;
use crate::page::PageId;
use crate::run::RunId;

mod links;

pub use links::{
    CRAWL_REQUEST_ENQUEUE_LINKS, CRAWL_REQUEST_LINKS, CrawlRequestEnqueueLinksParams,
    CrawlRequestLinks, CrawlRequestLinksParams, EnqueuedLink, EnqueuedLinks, MatchedLink,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The pattern of an HTTP method: one token of RFC 9110 §5.6.2.
pub const HTTP_METHOD_PATTERN: &str = "^[!#$%&'*+.^_`|~0-9A-Za-z-]+$";

/// The methods Fetch normalizes to uppercase, matched in any case.
const NORMALIZED_METHODS: [&str; 6] = ["DELETE", "GET", "HEAD", "OPTIONS", "POST", "PUT"];

/// The methods Fetch forbids a request to use, matched in any case.
const FORBIDDEN_METHODS: [&str; 3] = ["CONNECT", "TRACE", "TRACK"];

/// The method a crawl request is sent with (ADR 0093): an RFC 9110 token,
/// normalized as Fetch normalizes one, `GET` by default. A crawl request
/// is a read whatever its method, sent again on a retry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HttpMethod(String);

/// Why a text is no method a crawl request may be sent with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HttpMethodInvalid {
    /// It is no RFC 9110 token.
    #[error("a method is an RFC 9110 token, matching `{HTTP_METHOD_PATTERN}`")]
    NotAToken,
    /// Fetch forbids it.
    #[error(
        "a crawl request is never sent with `CONNECT`, `TRACE` or `TRACK`, which Fetch forbids"
    )]
    Forbidden,
}

impl HttpMethod {
    /// `GET`, a crawl request's method by default.
    pub fn get() -> Self {
        Self("GET".to_owned())
    }

    /// Reads `text` as a method, `DELETE`, `GET`, `HEAD`, `OPTIONS`, `POST`
    /// and `PUT` uppercased as Fetch normalizes them, any other kept as
    /// written, since methods are case-sensitive.
    ///
    /// # Errors
    ///
    /// When `text` is no token, or a method Fetch forbids.
    pub fn parse(text: &str) -> Result<Self, HttpMethodInvalid> {
        let is_token_character =
            |byte: &u8| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(byte);
        if text.is_empty() || !text.as_bytes().iter().all(is_token_character) {
            return Err(HttpMethodInvalid::NotAToken);
        }
        if FORBIDDEN_METHODS
            .iter()
            .any(|forbidden| forbidden.eq_ignore_ascii_case(text))
        {
            return Err(HttpMethodInvalid::Forbidden);
        }
        Ok(Self(
            NORMALIZED_METHODS
                .iter()
                .find(|normalized| normalized.eq_ignore_ascii_case(text))
                .map_or_else(|| text.to_owned(), |normalized| (*normalized).to_owned()),
        ))
    }

    /// The method as sent.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether it is `GET`.
    pub fn is_get(&self) -> bool {
        self.0 == "GET"
    }
}

impl Default for HttpMethod {
    fn default() -> Self {
        Self::get()
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for HttpMethod {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HttpMethod {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for HttpMethod {
    fn schema_name() -> Cow<'static, str> {
        "HttpMethod".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The method a crawl request is sent with (ADR 0093): an RFC 9110 token, `GET` by default; `DELETE`, `GET`, `HEAD`, `OPTIONS`, `POST` and `PUT` are uppercased as Fetch normalizes them, and `CONNECT`, `TRACE` and `TRACK` are refused.",
            "type": "string",
            "pattern": HTTP_METHOD_PATTERN,
        })
    }
}

/// The body a crawl request is sent with (ADR 0093): its bytes, in Base64
/// (RFC 4648, with padding) on the wire. Its hash is part of the request's
/// identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HttpBody(Vec<u8>);

/// Why a text is no body: it is no Base64 with padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a body is its bytes in Base64 (RFC 4648, with padding)")]
pub struct HttpBodyInvalid;

impl HttpBody {
    /// A body of `bytes`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Reads `text`, a body's bytes in Base64.
    ///
    /// # Errors
    ///
    /// When `text` is no Base64 with padding.
    pub fn from_base64(text: &str) -> Result<Self, HttpBodyInvalid> {
        data_encoding::BASE64
            .decode(text.as_bytes())
            .map(Self)
            .map_err(|_| HttpBodyInvalid)
    }

    /// Its bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for HttpBody {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&data_encoding::BASE64.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for HttpBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::from_base64(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for HttpBody {
    fn schema_name() -> Cow<'static, str> {
        "HttpBody".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The body a crawl request is sent with (ADR 0093): its bytes in Base64 (RFC 4648, with padding).",
            "type": "string",
            "contentEncoding": "base64",
        })
    }
}

/// Why the Frontier did not add a request (ADRs 0079, 0093 and 0120): an
/// open enumeration, so a reason a newer engine gives reads as received.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum NotAddedReason {
    /// The Frontier already admitted a request of the same identity: its
    /// method, its URL's key and its body's hash, or its declared `key`.
    Duplicate,
    /// Its URL is outside the crawl's scope (`crawl.scope.*`); or, for a
    /// nested sitemap, it is on a host its sitemap seed does not read
    /// sitemaps on.
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
    /// A sitemap nested more levels under its sitemap seed than
    /// `crawl.max_sitemap_nesting` (ADR 0093).
    MaxSitemapNesting,
    /// The first entry of `crawl.path_budgets` whose pattern its URL's path
    /// and query match already counted that entry's `max_requests`.
    PathBudget,
    /// Its URL's path falls under a trap the Frontier learned and closed
    /// (ADR 0005): a path pattern whose pages brought no new page and no
    /// URL outside it, which leaves a finished Run `unverified`.
    TrapPath,
    /// A sitemap entry no rule of its sitemap seed matches (ADR 0093).
    NoRule,
    /// On the browser tier, its link is one a person cannot see, a
    /// `nofollow` one among them: a trap a crawl never enters (ADR 0035).
    TrapLink,
    /// The `enqueueLinks` call that matched it already added its `limit`
    /// links (ADR 0093).
    LinkLimit,
    /// A reason a newer engine gives that this reader does not know, kept
    /// as received (ADR 0079).
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
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
    /// Whether it goes ahead of every request its domain holds pending,
    /// whatever their priority, as Crawlee's `forefront` (ADR 0005): of two
    /// such requests, the later goes first. A priority or `forefront` never
    /// orders requests of different domains (ADR 0004); `false` by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub forefront: bool,
    /// The method it is sent with: `GET` by default.
    #[serde(default, skip_serializing_if = "HttpMethod::is_get")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: HttpMethod,
    /// The body it is sent with, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: Option<HttpBody>,
    /// Its identity in the Frontier, declared by the program in place of
    /// its method, its URL's key and its body's hash: a canonicalization
    /// of the program's own, which always wins (ADR 0005).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: Option<String>,
    /// The page the attempt was given, whose document's base URL a relative
    /// `url` resolves against: its `<base href>`, resolved against the URL
    /// the document was reached at after its redirects, or that URL
    /// itself. Absent, for an attempt given no page, the crawl request's
    /// own URL is the base. A page no fetch tier serves fails a relative
    /// `url` with `closed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: Option<PageId>,
}

/// The notification that carries a batch of the seeds a Client streams to
/// a `run.add_seeds` call.
pub const SEEDS: &str = "$/seeds";

/// The notification that carries the answers of a batch of seeds a
/// `run.add_seeds` call admitted, in the order they came.
pub const ENQUEUED: &str = "$/enqueued";

/// How many seeds a `run.add_seeds` call takes before it answers any: the
/// credits it grants its Client at once (CO-02) **(shape)**. It grants one
/// more for each seed it answers, so it holds at most this many seeds at a
/// time, whatever the length of the stream.
pub const SEED_CREDITS: u32 = 256;

/// The attempt a seed is added from: a handler's `ctx.run`, which is bound
/// to its attempt (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeedAttempt {
    /// The ID of the crawl request whose attempt adds the seed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
}

/// What `run.add_seed` takes: one seed for a running crawl.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunAddSeedParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The seed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub seed: Seed,
    /// The attempt that adds it, when a handler adds it through its
    /// context's `run`; absent, the program adds it from outside any
    /// attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: Option<SeedAttempt>,
}

/// What `run.add_seeds` takes: the running crawl its Client streams seeds
/// to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunAddSeedsParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What `$/seeds` carries: the next seeds a Client streams to a
/// `run.add_seeds` call, never more than the credits the call granted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeedBatch {
    /// The id of the `run.add_seeds` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// The seeds, in order.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub seeds: Vec<Seed>,
    /// Whether they are the stream's last: the call answers once it
    /// answered them.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub end: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_false(flag: &bool) -> bool {
    !*flag
}

/// What `$/enqueued` carries: the answer of each seed of one `$/seeds`
/// batch, in the batch's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnqueuedBatch {
    /// The id of the `run.add_seeds` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// Each seed's answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub answers: Vec<Enqueued>,
}

/// What `run.add_seeds` answers once its stream ended: how many of the
/// seeds it took were added, and how many were not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunSeedsAdded {
    /// The seeds added.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: u64,
    /// The seeds not added, each with its reason in its answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub not_added: u64,
}

/// `run.add_seed`.
pub const RUN_ADD_SEED: Operation = Operation {
    name: "run.add_seed",
    description: "Adds one seed to a running crawl, `run.addSeed`, committed at once, and answers as an enqueue does: added, with its crawl request's ID and its URL as parsed, or not added, with its reason (`duplicate`, `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is `invalid_url`. An idle `crawl.keep_alive` crawl takes it and goes on. Through a handler's context, naming its `attempt`, it is bound to that attempt: once the attempt ended it fails with `attempt_ended`. A Run this engine does not run as a live crawl, or a crawl already ending its Run, fails with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunAddSeedParams>,
    result: schema_of::<Enqueued>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.add_seeds`.
pub const RUN_ADD_SEEDS: Operation = Operation {
    name: "run.add_seeds",
    description: "Takes a stream of seeds for a running crawl, `run.addSeeds`, and answers each as `run.add_seed` does, in order, so a million seeds never sit in memory. The engine grants the call 256 credits with `$/credit`, one seed each; the Client sends seeds in `$/seeds {id, seeds, end?}` batches, never more than its credits, and the engine commits each batch, sends its answers in one `$/enqueued {id, answers}` and grants as many credits again. Once it answered the batch marked `end`, the call answers how many seeds were added and how many were not. Cancelling it keeps the seeds already answered. A batch beyond the credits granted, or a seed that is no seed, fails the call with `invalid_params`; a Run this engine does not run as a live crawl, or a crawl already ending its Run, fails with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Sequence,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunAddSeedsParams>,
    result: schema_of::<RunSeedsAdded>,
    events: &[],
    cancellable: true,
    input: Some(StreamInput {
        notification: SEEDS,
        field: "seeds",
    }),
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

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
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl_request.enqueue`.
pub const CRAWL_REQUEST_ENQUEUE: Operation = Operation {
    name: "crawl_request.enqueue",
    description: "Adds one request to a crawl from inside a handler's attempt, a handler context's `enqueue`, and answers what the Frontier did with it: added, with its crawl request's ID and its URL as parsed, or not added, with its reason (`duplicate`, `out_of_scope`, `unsupported_scheme`, `invalid_url`, `max_depth`, `max_requests`, `path_budget` or a URL cap's name). A relative URL resolves against the document's base URL: its `<base href>`, resolved against the URL the document was reached at after its redirects, or that URL itself. A request is `duplicate` when one of its identity was already admitted: its method, its URL's key (the fragment, `utm_*`, `gclid`, `fbclid`, empty pairs and `;jsessionid=` dropped, the parameters sorted) and its body's hash, or the `key` the program declares in their place; it is still fetched at its own URL. The request sits one level deeper than the one whose attempt added it. An attempt whose lease no longer holds fails with `attempt_ended`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CrawlRequestEnqueueParams>,
    result: schema_of::<Enqueued>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The words of `crawl.scope.strategy`.
pub const SCOPE_STRATEGIES: &[&str] = &["same_hostname", "same_domain", "same_origin", "all"];

/// `crawl.scope.strategy` (ADRs 0093 and 0120).
pub const CRAWL_SCOPE_STRATEGY: Setting = Setting {
    name: "crawl.scope.strategy",
    description: "What a crawl's links are measured by against every seed, on every enqueue, redirect and Hook rewrite: `same_hostname` (Crawlee's default), `same_domain` (the registrable domain under the Public Suffix List), `same_origin` or `all`. One leading `www.` label is ignored, and under `same_origin` an `http` seed also admits its host's `https` origin on the same port. A link outside it is not added with `out_of_scope`. Fixed for a Run's life.",
    kind: ValueKind::Choice(SCOPE_STRATEGIES),
    default: same_hostname,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: true,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.scope.include` (ADRs 0081 and 0093).
pub const CRAWL_SCOPE_INCLUDE: Setting = Setting {
    name: "crawl.scope.include",
    description: "URL patterns, each a glob read as Playwright reads one and matched in the engine, of which a link must match one when there are any, besides `crawl.scope.strategy`, which they never replace. None by default. Fixed for a Run's life.",
    kind: ValueKind::List(&ValueKind::Text),
    default: no_patterns,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: true,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.scope.exclude` (ADRs 0081 and 0093).
pub const CRAWL_SCOPE_EXCLUDE: Setting = Setting {
    name: "crawl.scope.exclude",
    description: "URL patterns, each a glob read as Playwright reads one and matched in the engine, none of which a link may match, besides `crawl.scope.strategy` and `crawl.scope.include`. None by default. Fixed for a Run's life.",
    kind: ValueKind::List(&ValueKind::Text),
    default: no_patterns,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: true,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_depth` (ADRs 0005 and 0093; CO-07).
pub const CRAWL_MAX_DEPTH: Setting = Setting {
    name: "crawl.max_depth",
    description: "How deep a crawl goes, its seeds at depth 0 and each enqueue one level deeper than the request whose attempt added it: a deeper request is not added, with `max_depth`, and a Run that refused one ends `incomplete`. None by default; `null` clears it.",
    kind: ValueKind::Count,
    default: no_bound,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_requests` (ADRs 0005 and 0093; CO-08).
pub const CRAWL_MAX_REQUESTS: Setting = Setting {
    name: "crawl.max_requests",
    description: "How many requests a crawl admits, its seeds included: once it admitted this many, a further one is not added, with `max_requests`, and a Run that refused one ends `incomplete`. None by default; `null` clears it.",
    kind: ValueKind::Count,
    default: no_bound,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_url_length` (ADR 0120; pitfall U10).
pub const CRAWL_MAX_URL_LENGTH: Setting = Setting {
    name: "crawl.max_url_length",
    description: "The longest URL a crawl admits, in characters as the WHATWG parser serializes it, without its fragment (Scrapy's `URLLENGTH_LIMIT`): a longer one is not added, with `max_url_length`, and a finished Run that refused one is `unverified`. 2,083 by default; `null` turns the cap off.",
    kind: ValueKind::Count,
    default: max_url_length,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_path_segments` (ADR 0120; pitfall U10).
pub const CRAWL_MAX_PATH_SEGMENTS: Setting = Setting {
    name: "crawl.max_path_segments",
    description: "The most segments a URL's path may have, as the WHATWG parser splits it, an empty last segment (a trailing slash) not counted (Heritrix's `maxPathDepth`): a URL with more is not added, with `max_path_segments`, and a finished Run that refused one is `unverified`. 20 by default; `null` turns the cap off.",
    kind: ValueKind::Count,
    default: max_path_segments,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_segment_repeats` (ADR 0120; pitfall U10).
pub const CRAWL_MAX_SEGMENT_REPEATS: Setting = Setting {
    name: "crawl.max_segment_repeats",
    description: "How many times back to back a segment of a URL's path, or a run of segments, may appear (Heritrix's `maxRepetitions`), so `/a/a` passes and `/a/a/a` and `/a/b/a/b/a/b` do not, while numbers that recur apart, as in `/page/1/tag/1/sort/1`, do: a URL past it is not added, with `max_segment_repeats`, and a finished Run that refused one is `unverified`. 2 by default; `null` turns the cap off.",
    kind: ValueKind::Count,
    default: max_segment_repeats,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_sitemap_nesting` (ADR 0093's note of 2026-11-10).
pub const CRAWL_MAX_SITEMAP_NESTING: Setting = Setting {
    name: "crawl.max_sitemap_nesting",
    description: "How many levels of sitemaps a sitemap seed reads under the sitemap or robots.txt it names, each sitemap an index or a robots.txt names one level deeper than it: a sitemap nested deeper is not fetched, the seed's report keeps its URL with `max_sitemap_nesting`, and a Run that refused one ends `incomplete`. 5 by default; `null` turns the bound off.",
    kind: ValueKind::Count,
    default: max_sitemap_nesting,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// One entry of `crawl.path_budgets` (CO-08): a pattern, a glob read as
/// Playwright reads one and matched in the engine against a URL's path and
/// query, and how many requests it admits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PathBudget {
    /// The glob, such as `/shop?*`, matched against the path and query.
    pub pattern: String,
    /// How many requests whose URL it matches first the crawl admits.
    pub max_requests: u64,
}

impl crate::catalog::SettingJson for PathBudget {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// The fields of a `crawl.path_budgets` entry.
const PATH_BUDGET_FIELDS: [&str; 2] = ["pattern", "max_requests"];

impl PathBudget {
    /// Reads one entry of `crawl.path_budgets`.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message.
    pub fn read(value: &Value) -> Result<Self, String> {
        let Some(entry) = value.as_object() else {
            return Err("a path budget is a table of `pattern` and `max_requests`".to_owned());
        };
        if let Some(unknown) = entry
            .keys()
            .find(|field| !PATH_BUDGET_FIELDS.contains(&field.as_str()))
        {
            return Err(format!(
                "a path budget takes `pattern` and `max_requests`, not `{unknown}`"
            ));
        }
        let Some(pattern) = entry.get("pattern").and_then(Value::as_str) else {
            return Err("a path budget's `pattern` is a glob, such as `/shop?*`".to_owned());
        };
        let Some(max_requests) = entry.get("max_requests").and_then(Value::as_u64) else {
            return Err("a path budget's `max_requests` is a whole number from 0".to_owned());
        };
        Ok(Self {
            pattern: pattern.to_owned(),
            max_requests,
        })
    }

    /// The entries of a `crawl.path_budgets` value, in order, skipping any
    /// that is not one, which the settings refused before.
    pub fn list(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::read(entry).ok())
            .collect()
    }
}

/// A value of `crawl.path_budgets`.
pub const PATH_BUDGET_SHAPE: ValueShape = ValueShape {
    name: "PathBudget",
    check: |value| PathBudget::read(value).map(drop),
    schema: || {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": PATH_BUDGET_FIELDS,
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "A glob read as Playwright reads one, such as `/shop?*`, matched in the engine against a URL's path and query.",
                },
                "max_requests": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "How many requests whose URL the pattern matches first the crawl admits.",
                },
            },
        })
    },
    examples: || vec![json!({ "pattern": "/shop?*", "max_requests": 100 })],
};

/// `crawl.path_budgets` (CO-08; Spider's `with_budget`).
pub const CRAWL_PATH_BUDGETS: Setting = Setting {
    name: "crawl.path_budgets",
    description: "Budgets for sections of a site, each a table of `pattern`, a glob read as Playwright reads one and matched in the engine against a URL's path and query, such as `/shop?*`, and `max_requests`: the first entry whose pattern a request's URL matches counts it, and once that entry counted `max_requests`, a further request it matches first is not added, with `path_budget`, and a Run that refused one ends `incomplete`, its reasons naming the pattern and the count. A later entry the URL also matches does not count it. None by default.",
    kind: ValueKind::List(&ValueKind::Shape(&PATH_BUDGET_SHAPE)),
    default: no_patterns,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.keep_url_fragment` (ADR 0120; pitfall U05).
pub const CRAWL_KEEP_URL_FRAGMENT: Setting = Setting {
    name: "crawl.keep_url_fragment",
    description: "Whether a crawl request's identity keeps its URL's fragment, so a hash-routed app's views (`#/a`, `#/b`) are separate requests, set for the Run or for one of its Labels, whose own value wins for its requests (Crawlee's `keepUrlFragment`). Off by default: one URL with two fragments is one request. Only the tiers that run the page's scripts serve such a request: under `fetch.tier: auto` it runs on `browser`, and forcing `http`, `browser_fetch` or `discovered_api` where it is on fails the crawl's start with `setting_not_applicable`. Fixed for a Run's life: a start under the Run's ID that changes it is refused with `setting_not_applicable`.",
    kind: ValueKind::Boolean,
    default: no_fragment,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: true,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn no_fragment() -> Value {
    Value::Bool(false)
}

/// `frontier.learning` (ADRs 0005 and 0138).
pub const FRONTIER_LEARNING: Setting = Setting {
    name: "frontier.learning",
    description: "Whether the Frontier learns URL rules from the crawl's pages (ADR 0005): each fetched page's Page signature and `rel=canonical`, read with one run of the main-content step, teach it the query parameters with no effect, the equivalent forms, the traps and the pagination that merge or refuse the URLs they show to be one page. On by default. Off, no page is signed and no rule is learned, so the Run report's `learned_rules` is empty and its warning `learning_off` says why, while the fixed URL rules and the check for an exact duplicate stay; it is recorded in the Audit log and listed in the run's result.",
    kind: ValueKind::Boolean,
    default: learning_on,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(learning_switched_off),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The Audit log kind of a Run that switched `frontier.learning` off (ADR
/// 0138).
pub const LEARNING_SWITCHED_OFF: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "learning_switched_off",
    description: "A run switched `frontier.learning` off, once per run, with the key, its value, its built-in default and the layer that set it (ADR 0138): its Frontier learns no URL rule from its pages.",
    arrived_in: ARRIVED_IN,
};

fn learning_on() -> Value {
    Value::Bool(true)
}

fn learning_switched_off(value: &Value) -> bool {
    value == &Value::Bool(false)
}

/// `crawl.payload.max_size` (ADR 0082).
pub const CRAWL_PAYLOAD_MAX_SIZE: Setting = Setting {
    name: "crawl.payload.max_size",
    description: "The longest a crawl request's Payload may be, in bytes of its JSON as the engine writes it, compactly: a longer one is refused at its enqueue with `payload_invalid`, whose `limit` names this setting, since a Payload lives in the Frontier and in every Checkpoint. Large content goes to the Key-value store, and the Payload carries its key. 64 KiB (65,536 bytes) by default.",
    kind: ValueKind::Count,
    default: sixty_four_kibibytes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn sixty_four_kibibytes() -> Value {
    Value::from(65_536)
}

/// A Payload refused at its enqueue (ADR 0082).
pub const PAYLOAD_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "payload_invalid",
    description: "A crawl request's Payload was refused when it was enqueued, as a seed, an added seed or a handler's enqueue: it does not fit its Label's JSON Schema, each issue named by its path, or its JSON is longer than `crawl.payload.max_size`, which `limit` names. Nothing was added. Large content goes to the Key-value store, and the Payload carries its key.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A resume refused because a Label's Payload schema changed (ADR 0082).
pub const PAYLOAD_SCHEMA_CHANGED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "payload_schema_changed",
    description: "The crawl now declares a Label's Payload JSON Schema that the Run did not begin with, and Payloads of that Label's pending requests fail it, so the Run was not resumed and did no work. The details name the Label, how many pending Payloads fail out of how many, and the first failure. Give the new field a default in the schema, or start a new Run: a Payload is never migrated silently.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// One way a Payload does not fit its Label's JSON Schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PayloadIssue {
    /// Where in the Payload, a JSON Pointer: empty for the Payload itself.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// What is wrong there.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
}

/// What `payload_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PayloadInvalidDetails {
    /// The Label the Payload was enqueued under.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// Each way it does not fit the Label's JSON Schema; none when it is
    /// too long.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub issues: Vec<PayloadIssue>,
    /// The setting it is longer than, `crawl.payload.max_size`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<String>,
    /// Its JSON's length in bytes, when it is too long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: Option<u64>,
    /// The setting's value, when it is too long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_size: Option<u64>,
}

/// What `payload_schema_changed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PayloadSchemaChangedDetails {
    /// The Label whose Payload schema changed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// How many of its pending requests' Payloads fail the new schema.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failing: u64,
    /// How many of its requests are pending.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pending: u64,
    /// The first failing request's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// How its Payload fails the new schema.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub issues: Vec<PayloadIssue>,
}

fn same_hostname() -> Value {
    Value::from("same_hostname")
}

fn no_patterns() -> Value {
    Value::Array(Vec::new())
}

fn no_bound() -> Value {
    Value::Null
}

fn max_url_length() -> Value {
    Value::from(2_083)
}

fn max_path_segments() -> Value {
    Value::from(20)
}

fn max_segment_repeats() -> Value {
    Value::from(2)
}

fn max_sitemap_nesting() -> Value {
    Value::from(5)
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        RUN_SEEDS,
        RUN_ADD_SEED,
        RUN_ADD_SEEDS,
        CRAWL_REQUEST_ENQUEUE,
        CRAWL_REQUEST_LINKS,
        CRAWL_REQUEST_ENQUEUE_LINKS,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[PAYLOAD_INVALID, PAYLOAD_SCHEMA_CHANGED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        CRAWL_SCOPE_STRATEGY,
        CRAWL_SCOPE_INCLUDE,
        CRAWL_SCOPE_EXCLUDE,
        CRAWL_MAX_DEPTH,
        CRAWL_MAX_REQUESTS,
        CRAWL_MAX_URL_LENGTH,
        CRAWL_MAX_PATH_SEGMENTS,
        CRAWL_MAX_SEGMENT_REPEATS,
        CRAWL_MAX_SITEMAP_NESTING,
        CRAWL_PATH_BUDGETS,
        CRAWL_PAYLOAD_MAX_SIZE,
        CRAWL_KEEP_URL_FRAGMENT,
        FRONTIER_LEARNING,
    ],
    hook_stages: &[],
    shared_type_values: &[LEARNING_SWITCHED_OFF],
};

#[cfg(test)]
mod tests;
