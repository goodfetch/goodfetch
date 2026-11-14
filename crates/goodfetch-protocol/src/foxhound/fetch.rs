//! A crawl's fetch tiers (spec #579; ADRs 0020 and 0093): `fetch.tier`,
//! the tier that fetches a crawl request's document, and the tier
//! `crawl_request.handle` names when the engine fetched it; and the HTTP
//! tier's redundant requests (ADR 0040, RT-20): hedged requests,
//! `fetch.hedge.*`, and coalescing, `fetch.coalesce`; and `browser_required`,
//! a call a forced tier cannot serve exactly. Foxhound's own, since only a
//! crawl's requests are fetched by a tier.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
    ValueShape,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A tier that fetches a crawl request's document (ADR 0020), from the
/// cheapest up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FetchTier {
    /// An HTTP request, over reqwest in `standard` and `identified`
    /// Sessions.
    Http,
    /// The site's own API, found from its pages.
    DiscoveredApi,
    /// A request sent from a browser page's context.
    BrowserFetch,
    /// A browser that runs scripts without layout.
    LightBrowser,
    /// Collie's browser page.
    Browser,
}

impl FetchTier {
    /// Its name on the wire and in settings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::DiscoveredApi => "discovered_api",
            Self::BrowserFetch => "browser_fetch",
            Self::LightBrowser => "light_browser",
            Self::Browser => "browser",
        }
    }

    /// The tier `name` names, `None` for `auto` and any other text.
    pub fn named(name: &str) -> Option<Self> {
        TIERS.into_iter().find(|tier| tier.as_str() == name)
    }
}

const TIERS: [FetchTier; 5] = [
    FetchTier::Http,
    FetchTier::DiscoveredApi,
    FetchTier::BrowserFetch,
    FetchTier::LightBrowser,
    FetchTier::Browser,
];

/// The values `fetch.tier` takes: `auto`, then each tier.
pub const FETCH_TIERS: &[&str] = &[
    "auto",
    "http",
    "discovered_api",
    "browser_fetch",
    "light_browser",
    "browser",
];

fn auto() -> Value {
    Value::String("auto".to_owned())
}

/// `fetch.tier` (ADRs 0020 and 0093; spec #579).
pub const FETCH_TIER: Setting = Setting {
    name: "fetch.tier",
    description: "The tier that fetches each crawl request's document, set for the Run or for one of its Labels: `auto` (the default) lets the engine choose, whether set or left at the built-in default: it learns the tier per Label on each domain by sample comparison, the first `fetch.sample.count` requests (3) of a Label on a domain each run on `http` and then on `browser`, whose run commits (sent from one Session, or from two that share nothing where `fetch.http_from` sends the Label's HTTP from the transport), the Label moving to `http` once all of them staged the same Records (compared after Unicode NFC and whitespace normalization of their strings, in any order) and the same enqueues, the browser's run never matching where it ended on another URL or read its document in another `document.characterSet`, and staying on `browser` for the Run where its browser sample met a consent wall; split by the first segment of its URL paths when its samples disagree by segment (a segment whose own samples disagree stays on `browser`; at most 16 segments per Label and domain, the rest on `browser`), and on `browser` otherwise; a choice is sampled again as `fetch.sample.refresh_after` says; the choice is kept in the run file, so a resumed crawl takes it again, and Provenance records the tier that sent each attempt's document, `browser_fetch` where a document tier's request left from inside the Session's browser, as it does under `auto` unless `fetch.http_from` sends it from the Session's own transport, and whether it was forced; a request on `http` escalates to the browser at the first call that tier cannot serve exactly; `http` and `browser` force those tiers, nothing escalating, and a call the forced tier cannot serve fails with `browser_required`; a `standard` Session forced onto `http` sends from its own transport, which presents its browser's `User-Agent` and document request headers (where the browser is an installed one, such as the distribution's Chromium on Linux arm64 and musl, those of the version its executable prints for `--version`, read once per Run without starting a browser; none, with the Run warned `browser_version_unread`, where no browser is installed or it does not print a version and exit within 10 seconds), as do the reads of robots.txt, sitemaps and AI-use signal files that transport sends under every tier, and keeps the Session's cookies in a jar of its own, over reqwest's TLS fingerprint, not the browser's; `light_browser` forces the Lightpanda tier where `fetch.light_browser` allows it, and under `auto` a Run that allows it also runs a sample on `light_browser` where its `http` run differed, the Label moving to `light_browser` once every sample matched there or over `http`; under `auto`, a sample whose `http` run escalated at a network wait also runs on `discovered_api` once its browser run answered: the JSON calls the browser's page made, read as a page-number or cursor plan, are replayed over HTTP for the same request, a `page.wait_for_response` armed before a navigation of its page answered from the calls that navigation replays and any other escalating, and a Label whose samples all matched there, or over `http`, moves to `discovered_api`, its later requests replaying the plan for their own pages; `browser_fetch` forces the HTTP tier sent from inside the Session's browser context, the one tier that sends HTTP with the browser's own TLS fingerprint, its cookies the Session's, nothing escalating; forcing `discovered_api` is refused with `capability_unavailable` at the crawl's start until a forced tier takes its browser sample.",
    kind: ValueKind::Choice(FETCH_TIERS),
    default: auto,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.tier_race` (ADRs 0040 and 0093).
pub const FETCH_TIER_RACE: Setting = Setting {
    name: "fetch.tier_race",
    description: "Whether a crawl request races its tiers: browser-fetch first, then the navigation after the Label-on-domain's median browser-fetch latency, the first result that passes the Page check and the Label's comparison winning, both counting in Politeness and Usage. The loser is cancelled, and Provenance records the race. A race does not feed the sample comparison, so a request whose Label on its domain is due a sample takes the sample instead, and a request a hidden link reached races with no navigation. Set for the Run only: a Label setting it is refused with `setting_not_applicable`, as is a Run switching it on while `politeness.domain.concurrency` is 1, where the navigation could never start. Off by default.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.light_browser` (ADRs 0020 and 0093; spec #579).
pub const FETCH_LIGHT_BROWSER: Setting = Setting {
    name: "fetch.light_browser",
    description: "Whether a crawl may fetch on the `light_browser` tier: Lightpanda, run out of process, its build downloaded on first use into the Browser cache with its pinned digest and never bundled, so its AGPL licence stays outside the distribution. It runs the page's scripts without layout, and its page is the document they leave, so a wait for scripted content is served from it while a call that needs layout, pixels, a script of the handler's or a navigation escalates under `auto` and fails with `browser_required` under a forced tier. Set for the Run only. Off by default: `fetch.tier: light_browser` then fails the crawl's start with `capability_unavailable` naming this key, as it does on a platform Lightpanda builds nothing for.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// The values `fetch.http_from` takes: the default, then the two that
/// weaken browser-fetch for identity consistency (ADR 0023).
pub const HTTP_FROM_VALUES: &[&str] = &["browser", "learned", "transport"];

fn from_the_browser() -> Value {
    Value::String("browser".to_owned())
}

/// Whether a `fetch.http_from` sends a Label's HTTP from anywhere but the
/// Session's browser: a weakened control (ADR 0023).
fn not_from_the_browser(value: &Value) -> bool {
    value.as_str().is_some_and(|from| from != "browser")
}

/// `fetch.http_from` (ADRs 0020 and 0023; spec #2413).
pub const FETCH_HTTP_FROM: Setting = Setting {
    name: "fetch.http_from",
    description: "Where a Label's HTTP leaves from where the engine chooses its tier, under `fetch.tier: auto`, set for the Run or for one of its Labels; a forced `fetch.tier` ignores it. `browser` (the default) sends a sample's `http` run and a Label learned onto `http` from inside the Session's browser, recorded as `browser_fetch`, so the site meets one TLS fingerprint and one cookie jar. `transport` sends them from a Session's own HTTP transport, over its own TLS fingerprint and headers, recorded as `http`: a sample's `http` run and its `browser` run are drawn from two Sessions, one of each client, which share no cookie, storage or local address, and a Label learned onto `http` sends its documents, their redirect hops and its served pages' requests from a Session's own transport. `learned` behaves as `transport` until the engine learns per Label which client a site accepts. A choice learned under one value is sampled again under another that no longer allows its sender. `learned` and `transport` are a weakened control, named in the Run report's `weakened` and recorded in the Audit log as `feature_switched_off`.",
    kind: ValueKind::Choice(HTTP_FROM_VALUES),
    default: from_the_browser,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(not_from_the_browser),
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.encoding` (ADR 0119).
pub const FETCH_ENCODING: Setting = Setting {
    name: "fetch.encoding",
    description: "The encoding forced on each crawl request's own document after its BOM, a WHATWG Encoding label such as `shift_jis`, set for the Run or for one of its Labels; none by default. It stands where ADR 0119's order puts it: after the BOM, before the `Content-Type` charset, the `<meta>` prescan and UTF-8. It applies to the request's own document on every attempt, not to a handler's own navigations. A value that is no Encoding label is refused with `setting_invalid` when the crawl starts.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.kept_responses.urls` (ADR 0110).
pub const FETCH_KEPT_RESPONSES_URLS: Setting = Setting {
    name: "fetch.kept_responses.urls",
    description: "URL patterns, each a glob or a regular expression matched in the engine, naming the responses of a crawl request's own navigation kept for its handler's `keptResponse` and `keptResponses`, set for the Run or for one of its Labels, whose own list wins for its requests. None by default: nothing is kept. Only a tier that runs the page's scripts, or `discovered_api`, can keep them: forcing `http` or `browser_fetch` where a list is set fails the crawl's start with `setting_not_applicable`; any other list set is refused at the start with `capability_unavailable` until the engine keeps responses.",
    kind: ValueKind::List(&ValueKind::Text),
    default: no_patterns,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn no_patterns() -> Value {
    Value::Array(Vec::new())
}

fn null() -> Value {
    Value::Null
}

fn off() -> Value {
    Value::Bool(false)
}

fn on() -> Value {
    Value::Bool(true)
}

fn five() -> Value {
    Value::from(5_u64)
}

/// `fetch.hedge.enabled` (ADRs 0040 and 0086; RT-20).
pub const FETCH_HEDGE_ENABLED: Setting = Setting {
    name: "fetch.hedge.enabled",
    description: "Whether the HTTP tier hedges a crawl request: once a GET or HEAD from a work item that declares no Effect has not answered within its route's 95th-percentile latency, a second copy is sent on a new connection through the Session's own exit, and the first answer wins, the other cancelled. Both copies hold per-domain slots and count in Usage; a Paid fetch is never hedged. Off by default; a crawl that switches it on while `politeness.domain.concurrency` is 1 is refused with `setting_not_applicable`.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.hedge.max_percent` (ADR 0040; RT-20).
pub const FETCH_HEDGE_MAX_PERCENT: Setting = Setting {
    name: "fetch.hedge.max_percent",
    description: "The most hedges a run sends, as a whole percentage of the crawl requests its HTTP tier sent: 5 by default.",
    kind: ValueKind::Count,
    default: five,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.coalesce` (ADR 0040; RT-20).
pub const FETCH_COALESCE: Setting = Setting {
    name: "fetch.coalesce",
    description: "Whether identical GET or HEAD crawl requests in flight at once from one Session, from work items that declare no Effect, wait for one response: the same method, URL, headers and body. Never across Sessions or runs, never for a Paid fetch; Provenance records each response shared. On by default.",
    kind: ValueKind::Boolean,
    default: on,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// What `browser_required` details (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserRequiredDetails {
    /// The setting that forced the tier: `fetch.tier`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The tier it forced.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tier: FetchTier,
    /// The Label whose requests it forced, when it was set on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: Option<String>,
    /// The call the tier cannot serve exactly, by its operation's name, or
    /// the setting that judges the request's own document in its place:
    /// `session.page_check` for a page its markup cannot judge, and
    /// `fetch.encoding` for one only a guess could decode.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call: String,
    /// What serves it: another tier, and for a wait also content the page
    /// may simply lack.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A call a forced tier cannot serve exactly (ADR 0093).
pub const BROWSER_REQUIRED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "browser_required",
    description: "A handler's call on a page its forced fetch tier serves from a document, which that tier cannot serve exactly, such as a click, `evaluate` or a wait for content the document lacks: under a forced tier nothing escalates to the browser. The details name `fetch.tier`, the tier, the Label that forced it when one did, and the call; the fix names `fetch.tier: auto` or `browser`, and for a wait also content the page may simply lack. A request's own document that declares no encoding and is not UTF-8, which only the browser's guess reads, fails the same way under a forced tier (ADR 0119), its call `fetch.encoding` and its fix naming `fetch.encoding` and `fetch.tier`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `response_not_document` details (ADR 0119).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResponseNotDocumentDetails {
    /// The response's content type, as its `Content-Type` names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content_type: String,
    /// The call on the document, by its operation's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call: String,
    /// What reads such a response: the handler's `ctx.response`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A call on the document of a page whose response is no HTML document
/// (ADR 0119).
pub const RESPONSE_NOT_DOCUMENT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "response_not_document",
    description: "A handler's call that reads, waits on or acts on the document of a page whose accepted response is not an HTML document, such as a PDF its Label's `crawl.content_types` lists: a locator, `content()`, `formats`, `evaluate`, a screenshot or an AI verb. It fails at once on every tier and never escalates. The details name the response's content type and the call; the fix names `ctx.response`, which holds the body.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

fn nineteen() -> Value {
    Value::from(19_u64)
}

fn hundred_seconds() -> Value {
    Value::from(100_000_u64)
}

/// The most redirects `fetch.redirect.max` takes: as many as the pinned
/// browser follows in a navigation (ADR 0118).
pub const REDIRECT_MAX_LIMIT: u64 = 19;

/// `fetch.redirect.max` (ADR 0118; pitfall U12).
pub const FETCH_REDIRECT_MAX: Setting = Setting {
    name: "fetch.redirect.max",
    description: "How many redirects a crawl request's fetch follows, a refresh stub's hop counted as one: 19 by default, as many as the browser follows in a navigation, from 1 to 19; any other value is refused with `setting_invalid` when the crawl starts. One more fails the request with `too_many_redirects`, so a redirect loop costs a site this many requests and one more. To hand a redirect to the handler unfollowed, list its status in `crawl.handle_statuses`.",
    kind: ValueKind::Count,
    default: nineteen,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.refresh.max_delay` (ADR 0118; pitfall U13).
pub const FETCH_REFRESH_MAX_DELAY: Setting = Setting {
    name: "fetch.refresh.max_delay",
    description: "The longest delay of a refresh stub a crawl request's fetch follows: a `Refresh` header, or a `<meta http-equiv=\"refresh\">` outside `<noscript>`, to another URL with a delay up to this is followed at once as one more hop, counted in `fetch.redirect.max`. 100 s by default; `0` follows instant refreshes only, and `null` any delay. A refresh with no URL or to the document's own is never followed.",
    kind: ValueKind::Duration,
    default: hundred_seconds,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// The samples `fetch.sample.count` takes by default (ADR 0020; spec
/// #579).
pub const SAMPLE_COUNT: u64 = 3;

fn sample_count() -> Value {
    Value::from(SAMPLE_COUNT)
}

/// Whether a `fetch.sample.count` takes fewer samples than the built-in
/// three: a weakened control (ADR 0023).
fn fewer_samples(value: &Value) -> bool {
    value.as_u64().is_some_and(|count| count < SAMPLE_COUNT)
}

/// `fetch.sample.count` (ADRs 0020 and 0023; spec #579).
pub const FETCH_SAMPLE_COUNT: Setting = Setting {
    name: "fetch.sample.count",
    description: "How many of a Label's first requests on a domain are samples under `fetch.tier: auto`, each fetched on `http` and then on `browser`, whose run commits, from two Sessions that share nothing where `fetch.http_from` sends the Label's HTTP from the transport: the Label moves to `http` once all of them gave the same Records and enqueues. 3 by default; set for the Run only. Fewer than 3 is a weakened control, named in the Run report's `weakened`.",
    kind: ValueKind::Count,
    default: sample_count,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(fewer_samples),
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// The fields of a `fetch.sample.refresh_after` value.
const SAMPLE_REFRESH_FIELDS: &[&str] = &["requests", "after"];

/// When a Label's tier choice is sampled again: after it served
/// `requests`, or after `after` milliseconds, whichever comes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleRefresh {
    /// The requests a choice serves before it is sampled again.
    pub requests: u64,
    /// The milliseconds a choice stands before it is sampled again.
    pub after: u64,
}

impl SampleRefresh {
    /// The value `value` holds, or what is wrong with it.
    ///
    /// # Errors
    ///
    /// When `value` is no table of whole `requests` and `after`.
    pub fn read(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "`fetch.sample.refresh_after` is a table of `requests` and `after` (milliseconds), each a whole number: {error}"
            )
        })
    }
}

/// A value of `fetch.sample.refresh_after`.
pub const SAMPLE_REFRESH_SHAPE: ValueShape = ValueShape {
    name: "SampleRefresh",
    check: |value| SampleRefresh::read(value).map(drop),
    schema: || {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": SAMPLE_REFRESH_FIELDS,
            "properties": {
                "requests": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "The requests a Label's tier choice on a domain serves before it is sampled again.",
                },
                "after": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "The milliseconds a Label's tier choice on a domain stands before it is sampled again.",
                },
            },
        })
    },
    examples: || {
        vec![
            serde_json::json!({ "requests": 500, "after": 86_400_000 }),
            serde_json::json!({ "requests": 100, "after": 3_600_000 }),
        ]
    },
};

fn refresh_after() -> Value {
    serde_json::json!({ "requests": 500, "after": 86_400_000 })
}

/// `fetch.sample.refresh_after` (ADR 0020; spec #579).
pub const FETCH_SAMPLE_REFRESH_AFTER: Setting = Setting {
    name: "fetch.sample.refresh_after",
    description: "When a Label's tier choice on a domain under `fetch.tier: auto` is sampled again: after it served `requests` crawl requests, or `after` milliseconds after it was made, whichever comes first; 500 requests or 24 hours (86,400,000 ms) by default. Set for the Run only. A Label whose browser sample showed a consent wall stays on `browser` and is not sampled again.",
    kind: ValueKind::Shape(&SAMPLE_REFRESH_SHAPE),
    default: refresh_after,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[BROWSER_REQUIRED, RESPONSE_NOT_DOCUMENT],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        FETCH_TIER,
        FETCH_TIER_RACE,
        FETCH_LIGHT_BROWSER,
        FETCH_HTTP_FROM,
        FETCH_SAMPLE_COUNT,
        FETCH_SAMPLE_REFRESH_AFTER,
        FETCH_ENCODING,
        FETCH_HEDGE_ENABLED,
        FETCH_HEDGE_MAX_PERCENT,
        FETCH_COALESCE,
        FETCH_REDIRECT_MAX,
        FETCH_REFRESH_MAX_DELAY,
        FETCH_KEPT_RESPONSES_URLS,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
