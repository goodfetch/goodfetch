//! A crawl's fetch tiers (spec #579; ADRs 0020 and 0093): `fetch.tier`,
//! the tier that fetches a crawl request's document, and the tier
//! `crawl_request.handle` names when the engine fetched it. Foxhound's own,
//! since only a crawl's requests are fetched by a tier.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Authority, Entries, ProtocolVersion, Setting, ValueKind};

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
    description: "The tier that fetches each crawl request's document: `auto` (the default) lets the engine choose, and today hands the handler its request alone; `http` fetches it over HTTP and hands the handler a page served from the document; `discovered_api`, `browser_fetch`, `light_browser` and `browser` force those tiers, each refused with `capability_unavailable` at the crawl's start until it is built.",
    kind: ValueKind::Choice(FETCH_TIERS),
    default: auto,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[FETCH_TIER],
    hook_stages: &[],
    shared_type_values: &[],
};
