//! What a crawl's Frontier answers (ADRs 0079, 0093, 0102 and 0120): every
//! seed and every enqueue is answered with an `Enqueued` value, added with
//! its ID or not added with its reason, so a crawl that does nothing says
//! why. `run.seeds` lists the answers each seed of a crawl got when its Run
//! began, and `crawl_request.enqueue` adds a request from inside a
//! handler's attempt, resolving a relative URL against the document's base
//! URL. A crawl request is sent with a method and a body, which with its
//! URL's key make its identity, unless the program declares its `key`.
//! The `crawl.scope.*` settings say which links a crawl may follow, fixed
//! for the Run's life. Foxhound's own.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting,
    Stability, ValueKind, schema_of,
};
use crate::run::RunId;

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

/// Why the Frontier did not add a request (ADRs 0093 and 0120).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NotAddedReason {
    /// The Frontier already admitted a request of the same identity: its
    /// method, its URL's key and its body's hash, or its declared `key`.
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
    description: "Adds one request to a crawl from inside a handler's attempt, a handler context's `enqueue`, and answers what the Frontier did with it: added, with its crawl request's ID and its URL as parsed, or not added, with its reason (`duplicate`, `unsupported_scheme`, `invalid_url` or a cap's name). A relative URL resolves against the document's base URL: its `<base href>`, resolved against the URL the document was reached at after its redirects, or that URL itself. A request is `duplicate` when one of its identity was already admitted: its method, its URL's key (the fragment, `utm_*`, `gclid`, `fbclid`, empty pairs and `;jsessionid=` dropped, the parameters sorted) and its body's hash, or the `key` the program declares in their place; it is still fetched at its own URL. The request sits one level deeper than the one whose attempt added it. An attempt whose lease no longer holds fails with `attempt_ended`.",
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
    arrived_in: ARRIVED_IN,
};

fn same_hostname() -> Value {
    Value::from("same_hostname")
}

fn no_patterns() -> Value {
    Value::Array(Vec::new())
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_SEEDS, CRAWL_REQUEST_ENQUEUE],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        CRAWL_SCOPE_STRATEGY,
        CRAWL_SCOPE_INCLUDE,
        CRAWL_SCOPE_EXCLUDE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
