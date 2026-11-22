//! What a crawl's progress adds to every Run's (ADR 0102): its requests by
//! state, why its due requests wait, one group per reason, its requests in
//! flight, its sitemaps, whether a keep-alive crawl is idle, and its
//! concurrency. On the wire, a crawl's parts sit beside every Run's, which
//! a reader of [`RunProgress`] keeps among its `kind_parts`.

use crate::point_in_time::PointInTime;
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{KindPart, ProtocolVersion, schema_of};
use crate::governor::{AllowanceSource, GovernorLevel};
use crate::run::RunProgress;
use crate::settings::SettingLimit;

use super::report::{CountOf, SitemapReport};

/// A crawl's parts of `run.progress`, the operation's answer and the
/// event's payload for a crawl, which Foxhound's SDKs type as
/// [`CrawlRunProgress`]; the wire and Collie's `RunProgress` are every
/// Run's.
pub const CRAWL_RUN_PROGRESS: KindPart = KindPart {
    defined_in: module_path!(),
    entry: "run.progress",
    kind: "crawl",
    description: "A crawl's progress: every Run's, with its requests by state, why its due requests wait, its requests in flight, its sitemaps, whether a keep-alive crawl is idle, and its concurrency.",
    value: schema_of::<CrawlRunProgress>,
    arrived_in: ProtocolVersion { major: 0, minor: 1 },
};

/// The reason `politeness.domain.concurrency` holds a domain: as many of
/// its requests are in flight as its cap, or as a 429 lowered it to.
pub const DOMAIN_CONCURRENCY: &str = "domain_concurrency";
/// The reason `politeness.domain.delay` holds a domain: its last request
/// started less than the delay ago.
pub const DOMAIN_DELAY: &str = "domain_delay";
/// The reason a site's robots.txt `Crawl-delay` holds a domain.
pub const CRAWL_DELAY: &str = "crawl_delay";
/// The reason a domain backs off: the site's 429, 502, 503 or 504, a
/// connection refused or timed out, an HTTP/2 `ENHANCE_YOUR_CALM`, or a
/// `Retry-After` (ADR 0118); never a proxy's own answer (ADR 0053).
pub const BACKOFF: &str = "backoff";
/// The reason a domain waits because the shared Politeness file refused
/// its take, such as one busy past its timeout, until its Run tries it
/// again (ADR 0053).
pub const POLITENESS_FILE: &str = "politeness_file";
/// The reason a domain waits for its robots.txt, not served yet.
pub const ROBOTS_TXT: &str = "robots_txt";
/// The reason `crawl.handler.concurrency` handlers already run.
pub const HANDLER_CONCURRENCY: &str = "handler_concurrency";
/// The reason the Resource governor admits nothing new (ADR 0040).
pub const GOVERNOR: &str = "governor";
/// The reason a request waits for a Session of the pool, such as one
/// logging in.
pub const SESSION: &str = "session";
/// The reason a request waits behind a Human pause (ADR 0092).
pub const HUMAN_PAUSE: &str = "human_pause";

/// A crawl's progress (ADR 0102): what a crawl adds to every Run's, a part
/// no area supplied being absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRunProgress {
    /// Its requests by state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub requests: Option<RequestProgress>,
    /// Why its due requests wait, one group per reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub waiting: Option<Vec<WaitGroup>>,
    /// Each request being fetched or handled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub in_flight: Option<Vec<InFlightRequest>>,
    /// Each sitemap seed's sitemaps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sitemaps: Option<Vec<SitemapReport>>,
    /// Whether `crawl.keep_alive` holds it `running` with an empty Frontier
    /// and nothing in flight, until `run.add_seed` gives it work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub idle: Option<bool>,
    /// Its handlers running and the Resource governor's level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub concurrency: Option<CrawlConcurrency>,
    /// What every Run's progress holds.
    #[serde(flatten)]
    pub progress: RunProgress,
}

/// A crawl's requests by state (ADR 0102).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RequestProgress {
    /// How many wait to be fetched.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pending: u64,
    /// How many are being fetched.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fetching: u64,
    /// How many a handler handles.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub handling: u64,
    /// How many wait in a Human pause.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub in_human_pause: u64,
    /// How many succeeded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub succeeded: u64,
    /// How many failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failed: u64,
    /// How many were skipped.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub skipped: u64,
    /// How many a site blocked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub blocked: u64,
    /// The skipped ones, by skip reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by_skip_reason: Vec<CountOf>,
    /// The failed ones, by Error code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by_error_code: Vec<CountOf>,
    /// The blocked ones, by Block cause.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by_block_cause: Vec<CountOf>,
}

/// The due requests one reason holds (ADR 0102).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WaitGroup {
    /// The reason, such as [`DOMAIN_CONCURRENCY`]; a reason a newer engine
    /// sends is kept as it came.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// How many requests it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: u64,
    /// The domains it holds, at most [`HELD_DOMAINS`].
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<HeldDomain>,
    /// How many more domains it holds.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub more_domains: u64,
    /// The setting behind the wait, with its value and the layer that set
    /// it, as `deadline_exceeded`'s `limit` names one; none when no setting
    /// sets it, as for the governor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub setting: Option<SettingLimit>,
}

/// How many domains a wait group lists at most; the rest are counted.
pub const HELD_DOMAINS: usize = 10;

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// A domain a reason holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HeldDomain {
    /// The domain's key, its registrable domain unless `crawl.domain_keys`
    /// names another.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// When it is next admitted; none until a request in flight ends.
    #[serde(
        with = "crate::point_in_time::nullable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub next_admitted_at: Option<SystemTime>,
}

/// A request being fetched or handled (ADR 0102).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InFlightRequest {
    /// The request's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Where it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub phase: InFlightPhase,
    /// The fetch tier that fetches its document, when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tier: Option<String>,
    /// Its attempt, from 1.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: u32,
    /// When the attempt began.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub started_at: SystemTime,
}

/// Where a request in flight is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InFlightPhase {
    /// Its document is being fetched.
    Fetching,
    /// Its handler handles it.
    Handling,
    /// Its handler waits for a navigation gate's permit: a navigation of
    /// its page waits for its domain's Politeness, and its handler timeout
    /// does not run meanwhile (ADR 0093).
    WaitingForPermit,
}

/// A crawl's concurrency (ADRs 0040 and 0102).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlConcurrency {
    /// How many handlers run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub handlers: u64,
    /// `crawl.handler.concurrency`, per serving Connection.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub handler_concurrency: u64,
    /// The Resource governor's level, once it sampled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub governor: Option<GovernorLevel>,
    /// The browser pages open, once it sampled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub open_pages: Option<u32>,
    /// The browser pages the governor allows now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_allowance: Option<u32>,
    /// What set the Memory allowance behind it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub allowance_set_by: Option<AllowanceSource>,
}
