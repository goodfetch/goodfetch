//! A crawl's start and the exchange that serves its work items (ADRs 0003,
//! 0093 and 0102): `crawl.start` begins a crawl's Run from its seeds, and
//! the engine leases each work item to the program's handler as the handler
//! request `crawl_request.handle`, whose answer carries the Records the
//! attempt staged, committed once under its lease; `crawl.serve` offers
//! another Connection's handler to the same crawl, as `workers=N` handler
//! processes do. Foxhound's own.

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, HandlerRequest, Instrument, Metric,
    Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting, Stability,
    ValueKind, schema_of,
};
use crate::foxhound::dataset::DatasetDeclaration;
use crate::foxhound::fetch::FetchTier;
use crate::foxhound::frontier::{HttpBody, HttpMethod};
use crate::foxhound::hook::CrawlHookDeclaration;
use crate::foxhound::report::CrawlRunReport;
use crate::foxhound::session_pool::ServingSession;
use crate::page::read::PageField;
use crate::page::{FormatsRequest, PageId, PageResponse};
use crate::run::{RUN_ID_PATTERN, RunId, RunParams, RunStarted, Tags};
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a crawl starts from (ADR 0093): a request, or a sitemap seed, which
/// seeds the entries of a site's sitemaps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Seed {
    /// One request.
    Request(RequestSeed),
    /// The entries of a sitemap, or of the sitemaps a site's robots.txt
    /// names.
    Sitemap(SitemapSeed),
}

/// A sitemap seed (ADRs 0093 and 0120; CO-10): `sitemap` names a sitemap,
/// or a site's robots.txt (its path `/robots.txt`), whose `Sitemap:` lines
/// are read. The engine reads it, and every sitemap index it nests, once
/// the Run is running, through the domain policy, Politeness and robots.txt,
/// running no handler; each entry is checked as an enqueue is, at depth 0,
/// with the Label of the first rule whose pattern matches its URL and no
/// Payload, and one no rule matches is not added, with `no_rule`. The seed
/// widens the crawl's scope by the host of the URL it names, and an entry
/// outside the scope is not added, with `out_of_scope`, so a robots.txt
/// seed admits its own host's entries from a sitemap on another host it
/// names, and no other host's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SitemapSeed {
    /// The URL of the sitemap, or of the site's robots.txt.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sitemap: String,
    /// The rules that give each entry its Label, in order: the first whose
    /// pattern matches the entry's URL wins.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rules: Vec<SitemapRule>,
}

/// One rule of a sitemap seed: a URL pattern, a glob as
/// `crawl.scope.include` takes them, matched in the engine, and the Label
/// it gives the entries it matches, which take no Payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SitemapRule {
    /// The URL pattern.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: String,
    /// The Label of the entries it matches.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
}

/// One request a crawl starts from (ADR 0093): a URL, the Label that picks
/// its handler and its Payload, and how it is sent and identified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestSeed {
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
    /// Its priority among the requests of its domain: higher goes first,
    /// and a priority never orders requests of different domains (ADR
    /// 0004); 0 by default.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub priority: i64,
    /// Whether it goes ahead of every request its domain holds pending,
    /// whatever their priority, as Crawlee's `forefront` (ADR 0005): of two
    /// such requests, the later goes first; `false` by default.
    #[serde(default, skip_serializing_if = "is_false")]
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
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_false(flag: &bool) -> bool {
    !*flag
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_zero(priority: &i64) -> bool {
    *priority == 0
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

/// One Label a crawl declares (ADRs 0082 and 0093): its Payload's schema
/// and the settings its requests take over the Run's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelDeclaration {
    /// The JSON Schema its requests' Payloads are checked against, the
    /// side the program's schema parses from; none takes any Payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<Value>,
    /// Its settings, nested by key: those whose scopes include `label`
    /// (ADR 0093), such as `fetch.tier`, `session.page_check`, which judges
    /// its pages in place of the Run's, and `session.source`, which gives
    /// the Label a Session pool of its own; any other key is refused with
    /// `setting_not_applicable`.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, Value>,
}

/// The one Label a code-free crawl declares, which every request it
/// enqueues takes (ADR 0103).
pub const CODE_FREE_LABEL: &str = "page";

/// The one Dataset a code-free crawl declares, keyed by `url` (ADR 0103).
pub const CODE_FREE_DATASET: &str = "pages";

/// What a crawl with no handler reads from each page it fetches (spec
/// #582's code-free crawls; ADR 0103): the engine declares the Label
/// [`CODE_FREE_LABEL`] and the Dataset [`CODE_FREE_DATASET`], keyed by
/// `url`, whose JSON Schema is generated from `formats`, and commits one
/// Record per page fetched: the page's formats, as `page.formats` answers
/// them (its `url` the document's, after redirects), its response's
/// `status` and `fetched_at`, when the engine read it, and its named
/// `fields`, each read by a locator and given by a Value transform (ADR
/// 0123).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlExtraction {
    /// The formats each page's Record holds, as `page.formats` takes them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub formats: FormatsRequest,
    /// Which pages the crawl fetches beyond its seeds: `scope` by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub follow: ExtractionFollow,
    /// The named fields each page's Record holds beside its formats, by
    /// name, which no format's property and neither `status` nor
    /// `fetched_at` may take, each a value field, a list or rows read as
    /// `page.read` reads them (ADR 0147); a field the page does not give
    /// is null, and the Record's Provenance names why (`unread_fields`);
    /// none by default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: BTreeMap<String, PageField>,
}

/// Which pages a code-free crawl fetches beyond its seeds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionFollow {
    /// Every link of every page the crawl's scope admits: each `a[href]`
    /// of the page, admitted as `enqueueLinks` admits it.
    #[default]
    Scope,
    /// The entries of the sitemaps each seed's site names in its
    /// robots.txt, read as a sitemap seed of that robots.txt reads them,
    /// and no page's links.
    Sitemap,
    /// The seeds alone.
    None,
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
    /// The crawl's Labels, by name, each with its Payload's JSON Schema or
    /// none, and its own settings (ADRs 0082 and 0093): a scope between the
    /// Run and its requests. A Payload of a declared schema is checked
    /// against it at every enqueue. A seed, a sitemap rule or an enqueue
    /// naming a Label the crawl does not declare fails with
    /// `label_unknown`, and so does a start under the ID of a Run with
    /// pending requests of a Label no longer declared, before any work; a
    /// code-free crawl declares its own Label.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub labels: BTreeMap<String, LabelDeclaration>,
    /// The crawl's Datasets, each with its JSON Schema and its key (ADR
    /// 0103): a second of one name fails the start with
    /// `dataset_duplicate`, and a key on a field the schema does not
    /// require with `dataset_key_invalid`. Its handlers push Records only
    /// into these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub datasets: Vec<DatasetDeclaration>,
    /// What a crawl with no handler reads from each page (a code-free
    /// crawl, ADR 0103): the engine handles each request itself and
    /// declares the Label and the Dataset it commits into, so such a start
    /// declares no Dataset of its own, else it fails with
    /// `invalid_params`. None for a crawl whose program handles its
    /// requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub extraction: Option<CrawlExtraction>,
    /// The crawl's Hooks in code, in the order declared (ADR 0104). Never
    /// kept in the Run's state: a start that resumes the Run declares them
    /// again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hooks: Vec<CrawlHookDeclaration>,
    /// The Run's settings, nested by key: the run layer (ADR 0083).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
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
    /// The method it is sent with.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: HttpMethod,
    /// The body it is sent with, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: Option<HttpBody>,
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
    /// The Session of the crawl's Session pool that serves the attempt.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session: ServingSession,
    /// The tier that serves the request's page (`fetch.tier`): the engine
    /// fetches every request's document, so every request names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tier: Option<FetchTier>,
    /// The page served from the fetched document, which the handler reads
    /// as any page; every request names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: Option<PageId>,
    /// The response of the request's own document, when the engine
    /// fetched it: its final URL, status and headers as the site sent
    /// them, the encoding its document was decoded with, and the
    /// `request_id` whose body `response.body` reads (ADRs 0093 and 0119).
    /// None when the browser's navigation answered none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response: Option<PageResponse>,
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

/// A handler's skip of its request: `ctx.skip(message)` (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandlerSkip {
    /// The handler's own text.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
}

/// What a handler answers once its attempt returned: the outputs it staged,
/// which commit once, under the attempt's lease (ADR 0003), or its skip.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlRequestHandled {
    /// The Records it pushed, in order.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Vec<PushedRecord>,
    /// Its handler's skip: the request ends `skipped` with the reason
    /// `handler`, and the attempt's Records are discarded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub skipped: Option<HandlerSkip>,
}

/// `crawl.start`.
pub const CRAWL_START: Operation = Operation {
    name: "crawl.start",
    description: "Starts a crawl from its seeds, and answers once its Run is started. The engine then leases each work item to the program's handler as a `crawl_request.handle` request, and the crawl's Run finishes once every work item is done, or, with `crawl.keep_alive`, once `run.finish` lets it and every work item is done. A Run ID works as `run.start`'s.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Opener,
    reach: Reach::AllSurfaces,
    params: schema_of::<CrawlStartParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.serve`.
pub const CRAWL_SERVE: Operation = Operation {
    name: "crawl.serve",
    description: "Offers this Connection's handler to a live crawl another Connection started, as each of `workers=N` handler processes does (ADR 0024): the engine leases the crawl's work items to it as `crawl_request.handle` requests too, as many at once as `crawl.handler.concurrency` allows on each Connection. Answers the crawl's Run report once the Run ends; cancelling the call stops new work items, while those already sent still expect their answers. A Run this engine does not run as a crawl fails with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    // It serves a handler, which no CLI command, MCP tool or Code mode
    // global carries (ADR 0104).
    reach: Reach::SdkOnly,
    params: schema_of::<RunParams>,
    result: schema_of::<CrawlRunReport>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl_request.handle`.
pub const CRAWL_REQUEST_HANDLE: HandlerRequest = HandlerRequest {
    name: "crawl_request.handle",
    description: "Asks the program's handler to process one attempt of a work item under its lease. Its answer carries the Records the attempt staged, committed once while the lease holds, or the handler's skip, which ends the request `skipped`; a failure, or no answer before the lease ends, commits nothing, and the work item is retried under the Run's policy, while a failure with the Error code `blocked` ends it `blocked` with the Block cause its details name. A failure with the Error code `call_cancelled` by the program with nothing closed (details `{\"by\": \"program\"}`) gives the request back, the program's own stop (ADR 0093): it returns to pending with no attempt counted and is delivered again. When the lease ends first, the engine sends `$/cancel` for the request; an answer that still comes is a late commit, refused with `lease_lost`.",
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

/// A handler's own request to retry its attempt: `ctx.retry(message)`
/// (ADR 0093).
pub const RETRY_REQUESTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "retry_requested",
    description: "The handler asked for its attempt to run again, with `ctx.retry(message)`, whose message this is; the attempt committed nothing, and its request is retried under `crawl.retry.max`. When the retries run out, the request ends `failed` with this error.",
    group: ErrorGroup::Page,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A site failing for now, or a crawl request held unsent: past the bound
/// on its `Retry-After`, or with a start the Politeness file could not
/// record (ADRs 0093, 0118 and 0086).
pub const RESPONSE_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "response_unavailable",
    description: "The site is failing for now: its 5xx or 408 response stayed after the retries ran out, or its `Retry-After` asked for a time further away than `politeness.retry_after.max`, so nothing is sent to its domain before then and nothing waits for it. A request whose start could not be recorded for its domain's Politeness is held unsent with it too, since its delay could not be kept. The details name the response's `status` (`null` for a request held unsent), the time the site asked for as `retry_after` when it asked, and a `fix`. `run.retry_failed` re-admits the request once the site is back.",
    group: ErrorGroup::Page,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A response no handler is handed (ADRs 0093 and 0118).
pub const RESPONSE_NOT_OK: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "response_not_ok",
    description: "The response is not one a handler is handed: a 3xx that is not one of Fetch's redirect statuses (301, 302, 303, 307, 308), or a redirect whose `Location` is missing, does not parse or names no HTTP(S) URL. The details name the response's `url` and `status`, and a `fix` naming `crawl.handle_statuses`, which hands a status to the handler. A sitemap seed's sitemap (ADR 0093) fails with it on any status but a 2xx, 408, 429 or 5xx, and on a file that is no sitemap the crawl reads: XML that breaks off, another root, a gzip stream that does not decode, or more than sitemaps.org's 50,000 URLs or 50 MB uncompressed; its `details` name the `url`, and the `status` when one answered. Not retried: the site answers the same again.",
    group: ErrorGroup::Page,
    retryable: false,
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

/// An attempt whose handler's Connection ended (ADRs 0024 and 0093).
pub const CLIENT_GONE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "client_gone",
    description: "The Connection the attempt's request went on ended before its handler answered: the program or one of its `workers=N` handler processes ended, or its Connection dropped. Not the handler's own failure (`handler_failed`): the attempt committed nothing and is retried, on another Connection that serves the crawl, under `crawl.retry.max`.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Label declared twice, or handled twice (ADR 0093).
pub const LABEL_DUPLICATE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "label_duplicate",
    description: "The crawl declares a second Label of a name it already declares, or attaches a second handler to one Label, so the declaration failed at once, with no I/O. The details name the Label. Declare each Label once and attach one handler to it.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Label left without a handler (ADR 0093).
pub const LABEL_UNHANDLED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "label_unhandled",
    description: "The crawl declares a Label it attaches no handler to, and no default handler exists, so its Run was not started and nothing was sent. The details name the Label. Attach a handler to it with `crawl.handle`, or stop declaring it.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Label the crawl does not declare (ADRs 0093 and 0104).
pub const LABEL_UNKNOWN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "label_unknown",
    description: "A seed, an enqueue or a handler names a Label the crawl does not declare, such as another crawl's, so nothing was done; or a resume finds pending requests of a Label the crawl no longer declares, so the Run did no work. The details name the Label and those the crawl declares. Declare the Label on this crawl, or use one it declares.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `response_unavailable` details (ADR 0118).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResponseUnavailableDetails {
    /// The URL of the request; absent for a request Politeness held unsent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// The response's HTTP status; `null` for a request held unsent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: Option<u16>,
    /// The time the site's `Retry-After` asks for, in RFC 3339, read in
    /// either of its forms; `null` when it asks for none.
    #[serde(default, with = "crate::point_in_time::nullable")]
    #[schemars(with = "Option<String>", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub retry_after: Option<std::time::SystemTime>,
    /// What to do about it, such as re-admitting the request once the site
    /// is back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: Option<String>,
}

/// What `label_unknown` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LabelUnknownDetails {
    /// The Label's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// The names of the Labels the crawl declares, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub declared: Vec<String>,
}

/// A call on an attempt that already ended (ADR 0093).
pub const ATTEMPT_ENDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "attempt_ended",
    description: "The call names an attempt that already ended: its handler returned, it failed or was skipped, its lease ended, or its Run stopped. Nothing it asked for was done, so a forgotten `await` never silently loses a link.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A call that stages an output with an attempt while a declared Effect's
/// body runs in it (ADR 0106).
pub const EFFECT_STAGING_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "effect_staging_refused",
    description: "The call would stage an output with its attempt (an enqueue, a seed added through `ctx.run`, a Record or a Key-value store write through `ctx.run`) while a declared Effect's body runs in that attempt. A re-run that finds the Effect in the Effect journal does not run the body again, so the output would be lost for good; nothing was staged. The details' `fix` says to stage it after the declaration, from the output the body returned.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A call that would stage one more admission with an attempt that holds
/// `crawl.max_staged_enqueues` of them (ADR 0093).
pub const STAGING_FULL: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "staging_full",
    description: "The call would stage one more admission (an enqueue, an `enqueueLinks` call or a seed added through `ctx.run`) with an attempt that already staged `crawl.max_staged_enqueues` of them, which the attempt holds in the engine's memory until it commits. Nothing was staged. Enqueue many links with one `enqueueLinks` call, or raise the setting, which `limit` names.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `staging_full` names: the setting that bounds an attempt's staged
/// admissions, and its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StagingFullDetails {
    /// The setting's name, `crawl.max_staged_enqueues`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: String,
    /// How many admissions it lets one attempt stage.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max: u64,
}

/// A commit refused because its attempt's lease ended (ADR 0003).
pub const LEASE_LOST: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "lease_lost",
    description: "The attempt's lease ended before it committed: its handler answered after the lease's end, or the run file holds a later lease of its work item. Its outputs are discarded, and the commit of the work item's next attempt is the one that counts.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.handler` (ADRs 0080 and 0093).
pub const TIMEOUT_HANDLER: Setting = Setting {
    name: "timeout.handler",
    description: "How long a crawl's handler has for one attempt, which also bounds every action inside it, and from which a work item's lease is derived: the timeout plus the larger of 10 s and a tenth of it. 60 s, Crawlee's `requestHandlerTimeoutSecs`.",
    kind: ValueKind::Duration,
    default: one_minute,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn no_statuses() -> Value {
    Value::Array(Vec::new())
}

/// `crawl.handle_statuses` (ADRs 0093 and 0118).
pub const CRAWL_HANDLE_STATUSES: Setting = Setting {
    name: "crawl.handle_statuses",
    description: "Statuses whose responses go to the handler as they are: a redirect status listed here is not followed, and the handler gets the 3xx as its response. None by default.",
    kind: ValueKind::List(&ValueKind::Count),
    default: no_statuses,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn html_and_xhtml() -> Value {
    Value::Array(vec![
        Value::from("text/html"),
        Value::from("application/xhtml+xml"),
    ])
}

/// `crawl.content_types` (ADR 0119; pitfall H16).
pub const CRAWL_CONTENT_TYPES: Setting = Setting {
    name: "crawl.content_types",
    description: "The media types a Label's handler takes, each `type/subtype`, `type/*` or `*/*`, set for the Run or for one of its Labels, whose own list wins for its requests: `text/html` and `application/xhtml+xml` by default. A response that would reach the handler (a 2xx, or a status `crawl.handle_statuses` lists) of another type ends its request `skipped` with the reason `content_type` before its body is read, never retried. The type is the response's `Content-Type` essence, or, when it names none, the type the pinned Chromium's MIME sniffer gives its first 1,024 bytes; a response with neither a type nor a body is an empty HTML document. An accepted type that is not HTML or XHTML reaches the handler with its body in its response and a page holding no document, whose document calls fail with `response_not_document`. A handler's own navigations are not checked. A value that is no list of media ranges is refused with `setting_invalid` when the crawl starts.",
    kind: ValueKind::List(&ValueKind::Text),
    default: html_and_xhtml,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.retry.max` (ADR 0093).
pub const CRAWL_RETRY_MAX: Setting = Setting {
    name: "crawl.retry.max",
    description: "How many times a work item whose attempt failed retryably is tried again: 2, Scrapy's `RETRY_TIMES`.",
    kind: ValueKind::Count,
    default: two,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.max_staged_enqueues` (ADR 0093).
pub const CRAWL_MAX_STAGED_ENQUEUES: Setting = Setting {
    name: "crawl.max_staged_enqueues",
    description: "How many admissions one attempt may stage before it commits: its enqueues, `enqueueLinks` calls and seeds added through `ctx.run`, each held in the engine's memory with the attempt until its commit or its discard. One more fails with `staging_full`, and nothing is staged. 10,000 by default.",
    kind: ValueKind::Count,
    default: ten_thousand,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn ten_thousand() -> Value {
    Value::from(10_000_u64)
}

/// `crawl.retry.handler_failed` (ADR 0093).
pub const CRAWL_RETRY_HANDLER_FAILED: Setting = Setting {
    name: "crawl.retry.handler_failed",
    description: "Whether a work item whose handler failed with `handler_failed`, or whose Hook in code failed with `hook_failed` on the program's own exception, is tried again: `false`. A Label may set it for its requests.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.retry.engine_panicked` (ADR 0148).
pub const CRAWL_RETRY_ENGINE_PANICKED: Setting = Setting {
    name: "crawl.retry.engine_panicked",
    description: "Whether a work item whose attempt the engine's panic ended with `engine_panicked`, a bug in Goodfetch, is tried again, up to `crawl.retry.max`: `true`.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.handler.concurrency` (ADR 0093).
pub const CRAWL_HANDLER_CONCURRENCY: Setting = Setting {
    name: "crawl.handler.concurrency",
    description: "How many work items the program's handler is given at once: 16, Scrapy's `CONCURRENT_REQUESTS`.",
    kind: ValueKind::Count,
    default: sixteen,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.keep_alive` (ADRs 0037's RT-12 and 0102).
pub const CRAWL_KEEP_ALIVE: Setting = Setting {
    name: "crawl.keep_alive",
    description: "Whether a crawl's Run stays `running` with an empty Frontier and nothing in flight, waiting for new work, until `run.finish` lets it end once its work items are done: `false`, so a crawl finishes once its work items are done. Its idle time counts toward `timeout.run`, and an abort stops it as any Run.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.domain_keys` (ADR 0004).
pub const CRAWL_DOMAIN_KEYS: Setting = Setting {
    name: "crawl.domain_keys",
    description: "Host names, each with the domain key its requests and those of the hosts under it share, the longest name winning, in place of the registrable domain under the Public Suffix List: a Recipe's override of the key its per-domain queues use. None by default.",
    kind: ValueKind::Map(&ValueKind::Text),
    default: empty_table,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The orders a free request slot can go to a crawl's ready domain keys
/// in (ADR 0004's note of spec #2194).
pub const DOMAIN_ORDERS: &[&str] = &["fewest_in_flight", "round_robin"];

/// `crawl.domain_order` (ADR 0004's note of spec #2194).
pub const CRAWL_DOMAIN_ORDER: Setting = Setting {
    name: "crawl.domain_order",
    description: "Which ready site gets the next free request slot. `fewest_in_flight` picks the site with the fewest of this Run's requests in flight, so a slow site never holds every slot. `round_robin` takes each ready site in turn, one request each.",
    kind: ValueKind::Choice(DOMAIN_ORDERS),
    default: fewest_in_flight,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn fewest_in_flight() -> Value {
    Value::from("fewest_in_flight")
}

fn empty_table() -> Value {
    Value::Object(serde_json::Map::new())
}

fn one_minute() -> Value {
    Value::from(60_000_u64)
}

fn two() -> Value {
    Value::from(2_u64)
}

fn sixteen() -> Value {
    Value::from(16_u64)
}

fn no() -> Value {
    Value::Bool(false)
}

fn yes() -> Value {
    Value::Bool(true)
}

/// `goodfetch.work_item.leased`: the crawls' work items whose attempt runs
/// (ADR 0050).
pub const WORK_ITEM_LEASED: Metric = Metric {
    name: "goodfetch.work_item.leased",
    description: "The crawls' work items leased to a handler whose attempt has not ended.",
    instrument: Instrument::UpDownCounter,
    unit: "{work_item}",
    labels: &[],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.work_item.queued`: the crawls' work items waiting in their
/// per-domain queues (ADR 0050).
pub const WORK_ITEM_QUEUED: Metric = Metric {
    name: "goodfetch.work_item.queued",
    description: "The crawls' work items waiting in their per-domain queues to be admitted, each queue holding at most 64; those still in the Frontier past that bound are not counted.",
    instrument: Instrument::UpDownCounter,
    unit: "{work_item}",
    labels: &[],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.work_item.retried`: the attempts that ended to be tried
/// again (ADR 0050).
pub const WORK_ITEM_RETRIED: Metric = Metric {
    name: "goodfetch.work_item.retried",
    description: "The crawls' work items whose attempt ended and that wait to be tried again.",
    instrument: Instrument::Counter,
    unit: "{work_item}",
    labels: &[],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CRAWL_START, CRAWL_SERVE],
    events: &[],
    handler_requests: &[CRAWL_REQUEST_HANDLE],
    error_codes: &[
        HANDLER_FAILED,
        RETRY_REQUESTED,
        RESPONSE_UNAVAILABLE,
        RESPONSE_NOT_OK,
        HANDLER_LOST,
        CLIENT_GONE,
        LEASE_LOST,
        ATTEMPT_ENDED,
        LABEL_DUPLICATE,
        LABEL_UNHANDLED,
        LABEL_UNKNOWN,
        EFFECT_STAGING_REFUSED,
        STAGING_FULL,
    ],
    plugin_kinds: &[],
    metrics: &[WORK_ITEM_QUEUED, WORK_ITEM_LEASED, WORK_ITEM_RETRIED],
    settings: &[
        TIMEOUT_HANDLER,
        CRAWL_HANDLE_STATUSES,
        CRAWL_CONTENT_TYPES,
        CRAWL_RETRY_MAX,
        CRAWL_RETRY_HANDLER_FAILED,
        CRAWL_RETRY_ENGINE_PANICKED,
        CRAWL_MAX_STAGED_ENQUEUES,
        CRAWL_HANDLER_CONCURRENCY,
        CRAWL_DOMAIN_KEYS,
        CRAWL_DOMAIN_ORDER,
        CRAWL_KEEP_ALIVE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
