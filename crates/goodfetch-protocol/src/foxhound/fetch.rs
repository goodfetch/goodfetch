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
    description: "The tier that fetches each crawl request's document, set for the Run or for one of its Labels: `auto` (the default) lets the engine choose: set for the Run or a Label, it learns the tier per Label on each domain by sample comparison, the first 3 requests of a Label on a domain each run on `http` and then on `browser`, whose run commits, the Label moving to `http` once all 3 gave the same Records, split by the first segment of its URL paths when its samples disagree by segment (a segment whose own samples disagree stays on `browser`; at most 16 segments per Label and domain, the rest on `browser`), and on `browser` otherwise; the choice is kept in the run file, so a resumed crawl takes it again, and Provenance records each attempt's tier and whether it was forced; a request on `http` escalates to the browser at the first call that tier cannot serve exactly; left at the built-in default it still gives the handler its request alone; `http` and `browser` force those tiers, nothing escalating, and a call the forced tier cannot serve fails with `browser_required`; `discovered_api`, `browser_fetch` and `light_browser` are refused with `capability_unavailable` at the crawl's start until they are built.",
    kind: ValueKind::Choice(FETCH_TIERS),
    default: auto,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.tier_race` (ADRs 0040 and 0093).
pub const FETCH_TIER_RACE: Setting = Setting {
    name: "fetch.tier_race",
    description: "Whether a crawl request races its tiers: browser-fetch first, then the navigation after the Label-on-domain's median browser-fetch latency, the first result that passes the Page check and the Label's comparison winning, both counting in Politeness and Usage. Set for the Run only: a Label setting it is refused with `setting_not_applicable`. Off by default; switched on, the crawl's start is refused with `capability_unavailable` until the race is built.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `fetch.encoding` (ADR 0119).
pub const FETCH_ENCODING: Setting = Setting {
    name: "fetch.encoding",
    description: "The encoding forced on each crawl request's own document after its BOM, a WHATWG Encoding label such as `shift_jis`, set for the Run or for one of its Labels; none by default. A value that is no Encoding label is refused with `setting_invalid`; a valid one is refused with `capability_unavailable` at the crawl's start until every tier decodes in ADR 0119's order.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

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
    /// The call the tier cannot serve exactly, by its operation's name.
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
    description: "A handler's call on a page its forced fetch tier serves from a document, which that tier cannot serve exactly, such as a click, `evaluate` or a wait for content the document lacks: under a forced tier nothing escalates to the browser. The details name `fetch.tier`, the tier, the Label that forced it when one did, and the call; the fix names `fetch.tier: auto` or `browser`, and for a wait also content the page may simply lack.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[BROWSER_REQUIRED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        FETCH_TIER,
        FETCH_TIER_RACE,
        FETCH_ENCODING,
        FETCH_HEDGE_ENABLED,
        FETCH_HEDGE_MAX_PERCENT,
        FETCH_COALESCE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
