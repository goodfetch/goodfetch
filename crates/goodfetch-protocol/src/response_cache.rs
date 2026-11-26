//! The Response cache's contracts (ADR 0046): `cache.mode`, the bounds
//! `cache.max_size` and `cache.max_age_ceiling`, the Error code
//! `replay_not_allowed` a Schedule whose target sets `replay` is refused
//! with, and one entry of the cache's index as its format's schema
//! describes it. Both Products carry them, since the engine's interception
//! and every fetch tier consult the one cache. A user sees and empties the
//! cache with `cache.stats`, `cache.clear` and `cache.verify`.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
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
    /// Every response stored, POST, PUT and PATCH included, keyed by its
    /// request's canonical body, and served again with no request and no
    /// revalidation: the development loop.
    Replay,
}

impl CacheMode {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Off, Self::Revalidate, Self::Replay];

    /// Its name in settings.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }

    /// The mode `name` names; none for a name this minor does not know.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

/// The values of `cache.mode`.
pub const CACHE_MODES: &[&str] = &["off", "revalidate", "replay"];

fn off() -> serde_json::Value {
    serde_json::Value::String("off".to_owned())
}

/// `cache.mode` (ADRs 0028 and 0046).
pub const CACHE_MODE: Setting = Setting {
    name: "cache.mode",
    description: "What the per-user Response cache does: `off` (the default) stores and serves nothing, so a run never serves a stale page silently; `revalidate` stores GET and HEAD responses, serves a fresh one with no request and revalidates a stale one with `ETag` and `Last-Modified` under RFC 9111, serving the stored body on a 304; `replay`, for development, stores every response, whatever its method, status or freshness, and serves it again with no request and no revalidation. A response sent with or setting a cookie or a credential, or marked `private`, is kept in its Session's Saved identity's partition, or the Session's own when it has none, which no other Session reads. Every tier shares the cache: a browser page's requests are served through its interception, and a request a site's service worker answers never reaches it. The key is the method, the URL without its fragment, the request headers the response's `Vary` names and, under `replay`, the SHA-256 of the request's canonical body: a JSON body with its object keys sorted, a form with its fields sorted by name and each name's values in their order, a body sent with no `Content-Type` read as JSON or a form by its bytes; a `Vary: *` response is never stored. A Schedule's target never sets `replay`. An engine that finds a cache written by a newer format runs without it, reading and writing nothing. When a site's service worker answers a request the Response cache must serve, set `browser.service_workers` to `block` (ADR 0121).",
    kind: ValueKind::Choice(CACHE_MODES),
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
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
    unbuilt: None,
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
    unbuilt: None,
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
    /// The SHA-256 of the request's canonical body, as lower-case hex, for
    /// an entry `replay` stored; empty for one `revalidate` stored.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub replay_key: String,
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

/// What `cache.stats` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CacheStatsParams {}

/// What `cache.stats` answers: what the caller's Response cache holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CacheStats {
    /// The caller's entries.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: u64,
    /// The bodies they hold, each once however many entries share it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bodies: u64,
    /// The bytes those bodies weigh, each once.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body_bytes: u64,
    /// The bytes the whole cache holds on disk, every person's bodies and
    /// its index, which `cache.max_size` bounds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub held_bytes: u64,
    /// The caller's entries by partition, the most entries first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub partitions: Vec<CachePartitionStats>,
}

/// One partition's entries, as `cache.stats` counts them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CachePartitionStats {
    /// `public`, or the Saved identity whose partition it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub partition: String,
    /// The proxy's exit country its entries were fetched through; empty
    /// when they left directly or the Proxy declares none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub country: String,
    /// The Identity profile's language; empty without one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub language: String,
    /// The Identity profile's device class; empty without one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub device: String,
    /// Its entries.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: u64,
}

/// `cache.stats` (ADR 0046).
pub const CACHE_STATS: Operation = Operation {
    name: "cache.stats",
    description: "Counts the caller's entries in the Response cache of the engine's major, the bodies they hold and the bytes those weigh, by partition, and the bytes the whole cache holds against `cache.max_size`. A cache never written counts nothing and is not created; one of a format version the engine cannot read is refused with `format_version_unsupported`.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<CacheStatsParams>,
    result: schema_of::<CacheStats>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `cache.clear` takes: which of the caller's entries leave, each
/// given filter narrowing it; every one of them when none is given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CacheClearParams {
    /// Only the entries of this host and its subdomains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: Option<String>,
    /// Only the entries this Run fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// Only the entries of this partition: `public`, or a Saved
    /// identity's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub partition: Option<String>,
    /// Only the entries fetched through this proxy exit country; the
    /// empty text for those that left directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub country: Option<String>,
    /// Only the entries fetched with this Identity profile language; the
    /// empty text for those fetched without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub language: Option<String>,
    /// Only the entries fetched with this device class; the empty text
    /// for those fetched without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub device: Option<String>,
}

/// What `cache.clear` answers: what left.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CacheCleared {
    /// The entries deleted.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: u64,
    /// The body files deleted with them: those no entry left holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bodies: u64,
    /// The bytes those body files weighed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bytes: u64,
}

/// `cache.clear` (ADR 0046).
pub const CACHE_CLEAR: Operation = Operation {
    name: "cache.clear",
    description: "Deletes the caller's entries from the Response cache of the engine's major: those of a domain and its subdomains, of a Run, of a partition, or every one when no filter is given; given filters narrow each other. A body file leaves with the last entry holding it, and nothing of a deleted entry stays in the cache's files. A cache never written is not created; one of a format version the engine cannot read is refused with `format_version_unsupported`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<CacheClearParams>,
    result: schema_of::<CacheCleared>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `cache.verify` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CacheVerifyParams {}

/// What `cache.verify` answers: what it read, and what it found damaged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CacheVerified {
    /// The caller's entries it checked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: u64,
    /// The body files it read and hashed, each once.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bodies: u64,
    /// The entries whose body file is missing or no longer the body
    /// stored; none when the cache is whole.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub damaged: Vec<CacheDamage>,
}

/// An entry whose body file `cache.verify` found damaged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CacheDamage {
    /// The entry's partition: `public`, or a Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub partition: String,
    /// The request's method.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The request's URL, without its fragment.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The Run that fetched it; none outside a Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// The SHA-256 of the body stored, as lower-case hex: its file's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body_sha256: String,
    /// What is wrong with its body file.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub problem: CacheBodyProblem,
}

/// What `cache.verify` found wrong with a body file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheBodyProblem {
    /// The file is not there.
    Missing,
    /// The file's bytes are not the body stored: their SHA-256 or their
    /// length is another.
    Changed,
}

/// `cache.verify` (ADR 0046).
pub const CACHE_VERIFY: Operation = Operation {
    name: "cache.verify",
    description: "Reads every body file the caller's entries in the Response cache of the engine's major hold, and lists each entry whose file is missing or whose bytes no longer hash to the body stored; it changes nothing, and `cache.clear` deletes what it lists. A cache never written holds nothing to verify and is not created; one of a format version the engine cannot read is refused with `format_version_unsupported`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<CacheVerifyParams>,
    result: schema_of::<CacheVerified>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CACHE_STATS, CACHE_CLEAR, CACHE_VERIFY],
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
