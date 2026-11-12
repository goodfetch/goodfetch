//! A crawl's Session pool (ADRs 0006, 0021, 0035 and 0088): the Session of
//! the pool that serves each attempt, which `crawl_request.handle` names
//! with its proxy tier and whether its first page after a resume
//! revalidates it, and the `session.pool.*` keys that bound how long a
//! Session serves. Foxhound's own, since only a crawl has a Session pool.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Authority, Entries, ProtocolVersion, Setting, ValueKind};
use crate::page::SessionId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A proxy tier of ADR 0021's ladder, from the cheapest up: a Session
/// binds one proxy of its tier, and a block that escalates moves its
/// domain one tier up, its replacement opening there.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ProxyTier {
    /// No proxy: the machine's own address.
    None,
    /// A datacenter proxy.
    Datacenter,
    /// An ISP proxy.
    Isp,
    /// A residential proxy.
    Residential,
    /// A mobile proxy, the top of the ladder.
    Mobile,
}

impl ProxyTier {
    /// Its name on the wire and in run state.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Datacenter => "datacenter",
            Self::Isp => "isp",
            Self::Residential => "residential",
            Self::Mobile => "mobile",
        }
    }

    /// The tier named `name`, if any.
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::None,
            Self::Datacenter,
            Self::Isp,
            Self::Residential,
            Self::Mobile,
        ]
        .into_iter()
        .find(|tier| tier.as_str() == name)
    }

    /// The tier one up the ladder; the top stays where it is.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::None => Self::Datacenter,
            Self::Datacenter => Self::Isp,
            Self::Isp => Self::Residential,
            Self::Residential | Self::Mobile => Self::Mobile,
        }
    }
}

/// The Session of the crawl's Session pool that serves one attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServingSession {
    /// Its ID within the Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: SessionId,
    /// The proxy tier it opened at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_tier: ProxyTier,
    /// Whether this is its first page since the crawl resumed, which
    /// revalidates the Session's login before the page is trusted.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revalidate: bool,
}

fn one_hundred() -> Value {
    Value::from(100_u64)
}

fn fifty() -> Value {
    Value::from(50_u64)
}

fn fifty_minutes() -> Value {
    Value::from(3_000_000_u64)
}

fn three() -> Value {
    Value::from(3_u64)
}

/// `session.pool.max_size` (ADR 0006).
pub const SESSION_POOL_MAX_SIZE: Setting = Setting {
    name: "session.pool.max_size",
    description: "How many Sessions a crawl's Session pool keeps open at once: 100. A request goes to a new Session while the pool has room, then to one drawn among those open.",
    kind: ValueKind::Count,
    default: one_hundred,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `session.pool.max_uses` (ADRs 0006 and 0035).
pub const SESSION_POOL_MAX_USES: Setting = Setting {
    name: "session.pool.max_uses",
    description: "How many requests one Session of a crawl's Session pool serves before it is retired: 50, Crawlee's `maxUsageCount`.",
    kind: ValueKind::Count,
    default: fifty,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `session.pool.max_age` (ADRs 0006 and 0035).
pub const SESSION_POOL_MAX_AGE: Setting = Setting {
    name: "session.pool.max_age",
    description: "How long after it opened a Session of a crawl's Session pool is retired: 3,000 s, Crawlee's `maxAgeSecs`.",
    kind: ValueKind::Duration,
    default: fifty_minutes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `session.pool.max_error_score` (ADR 0006).
pub const SESSION_POOL_MAX_ERROR_SCORE: Setting = Setting {
    name: "session.pool.max_error_score",
    description: "The error score at which a Session of a crawl's Session pool is retired: 3. A block or a failed Page check adds 1, a success takes 0.5 off, never below 0 (Crawlee's model).",
    kind: ValueKind::Count,
    default: three,
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
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        SESSION_POOL_MAX_SIZE,
        SESSION_POOL_MAX_USES,
        SESSION_POOL_MAX_AGE,
        SESSION_POOL_MAX_ERROR_SCORE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
