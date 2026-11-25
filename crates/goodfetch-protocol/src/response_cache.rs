//! The Response cache's contracts (ADR 0046): `cache.mode`, the bounds
//! `cache.max_size` and `cache.max_age_ceiling`, the Error code
//! `replay_not_allowed` a Schedule whose target sets `replay` is refused
//! with, and one entry of the cache's index as its format's schema
//! describes it. Both Products carry them, since the engine's interception
//! and every fetch tier consult the one cache.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What the Response cache does, `cache.mode`'s value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// Nothing stored, nothing served, the cache never opened.
    Off,
    /// GET and HEAD responses stored, a fresh one served with no request
    /// and a stale one revalidated under RFC 9111.
    Revalidate,
}

impl CacheMode {
    /// Every value, in declaration order.
    pub const ALL: [Self; 2] = [Self::Off, Self::Revalidate];

    /// Its name in settings.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }

    /// The mode `name` names; none for a name this minor does not know.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

/// The values of `cache.mode` this minor builds: `replay` joins them with
/// the ticket that builds it (#794).
pub const CACHE_MODES: &[&str] = &["off", "revalidate"];

fn off() -> serde_json::Value {
    serde_json::Value::String("off".to_owned())
}

/// `cache.mode` (ADRs 0028 and 0046).
pub const CACHE_MODE: Setting = Setting {
    name: "cache.mode",
    description: "What the per-user Response cache does: `off` (the default) stores and serves nothing, so a run never serves a stale page silently; `revalidate` stores GET and HEAD responses sent with no cookie or credential, whose response sets no cookie and is not `private`, serves a fresh one with no request and revalidates a stale one with `ETag` and `Last-Modified` under RFC 9111, serving the stored body on a 304. The key is the method, the URL without its fragment and the request headers the response's `Vary` names; a `Vary: *` response is never stored. An engine that finds a cache written by a newer format runs without it, reading and writing nothing. When a site's service worker answers a request the Response cache must serve, set `browser.service_workers` to `block` (ADR 0121).",
    kind: ValueKind::Choice(CACHE_MODES),
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("response_cache"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn ten_gigabytes() -> serde_json::Value {
    serde_json::json!(10_000_000_000_u64)
}

/// `cache.max_size` (ADR 0046).
pub const CACHE_MAX_SIZE: Setting = Setting {
    name: "cache.max_size",
    description: "The most bytes the Response cache keeps for a user, its index and its bodies together: 10 GB, beyond which the least recently used entries are evicted. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: ten_gigabytes,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("response_cache_bounds"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn thirty_days() -> serde_json::Value {
    serde_json::json!(30_u64 * 24 * 60 * 60 * 1000)
}

/// `cache.max_age_ceiling` (ADR 0046).
pub const CACHE_MAX_AGE_CEILING: Setting = Setting {
    name: "cache.max_age_ceiling",
    description: "How long the Response cache keeps an entry, whatever its freshness: 30 days, after which it is deleted in every mode. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: thirty_days,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("response_cache_bounds"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Schedule whose target sets `cache.mode: "replay"` (ADRs 0036 and
/// 0046).
pub const REPLAY_NOT_ALLOWED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "replay_not_allowed",
    description: "A Schedule's target sets `cache.mode` to `replay`, whose replayed pages would hide a change from the Baseline. Set `revalidate` or `off` for a scheduled target.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// One entry of the Response cache's index, as its rows hold it (ADR
/// 0046): the request it answers, the response's head, where its body
/// lies and which Run fetched it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResponseCacheEntry {
    /// Its partition: `public` for a response sent with no cookie or
    /// credential, which set no cookie and is not `private`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub partition: String,
    /// The person it was fetched for, on a shared Daemon the partition's
    /// person (ADR 0031).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub person: String,
    /// The proxy's exit country it was fetched through, as the Proxy
    /// declares it; empty when it left directly or the Proxy declares none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub country: String,
    /// The Identity profile's language it was fetched with; empty without
    /// one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub language: String,
    /// The Identity profile's device class it was fetched with, `desktop`
    /// or `mobile`; empty without one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub device: String,
    /// The request's method.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The request's URL, without its fragment.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The SHA-256 of the request headers the response's `Vary` names, as
    /// lower-case hex: the variant of the URL it answers.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub vary_key: String,
    /// The request headers the response's `Vary` names, and the request's
    /// `host`, as the request sent them: names in lower case.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_headers: Vec<(String, String)>,
    /// The response's status.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// The response's headers, names in lower case, in the order received.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response_headers: Vec<(String, String)>,
    /// When the response was received, which its freshness counts from.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub fetched_at: SystemTime,
    /// The SHA-256 of its body, as lower-case hex: the name of its body
    /// file.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body_sha256: String,
    /// Its body's length in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body_size: u64,
    /// The Run that fetched it; none outside a Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// When it was last stored or served.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub used_at: SystemTime,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[REPLAY_NOT_ALLOWED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[CACHE_MODE, CACHE_MAX_SIZE, CACHE_MAX_AGE_CEILING],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
