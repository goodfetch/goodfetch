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
use crate::page::proxy::{PROXY_KINDS, ProxyKind};

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

    /// The tier corresponding to `kind`.
    #[must_use]
    pub const fn of_kind(kind: ProxyKind) -> Self {
        match kind {
            ProxyKind::Datacenter => Self::Datacenter,
            ProxyKind::Isp => Self::Isp,
            ProxyKind::Residential => Self::Residential,
            ProxyKind::Mobile => Self::Mobile,
        }
    }

    /// The proxy kind this tier names; `None` for [`Self::None`].
    #[must_use]
    pub const fn kind(self) -> Option<ProxyKind> {
        match self {
            Self::None => None,
            Self::Datacenter => Some(ProxyKind::Datacenter),
            Self::Isp => Some(ProxyKind::Isp),
            Self::Residential => Some(ProxyKind::Residential),
            Self::Mobile => Some(ProxyKind::Mobile),
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
    description: "How many Sessions a crawl's Session pool keeps open at once, whether each sends from its own HTTP transport or from its browser: 100. A request goes to an open Session of the client it needs at its domain's proxy tier that has room under `session.pool.session_concurrency`, a new Session opening only when none has room and the pool is under this size; a full pool draws among the busy ones.",
    kind: ValueKind::Count,
    default: one_hundred,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    deprecated: None,
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
    deprecated: None,
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
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The rules by which a crawl's Session pool opens a new Session (spec
/// #2139).
pub const GROWTHS: &[&str] = &["on_demand", "eager"];

fn on_demand() -> Value {
    Value::from("on_demand")
}

/// `session.pool.growth` (spec #2139, ADR 0006).
pub const SESSION_POOL_GROWTH: Setting = Setting {
    name: "session.pool.growth",
    description: "When a crawl's Session pool opens a new Session. `on_demand` opens one only when no open Session of the client a request needs, at its domain's proxy tier, has room under `session.pool.session_concurrency`, or when a block that retired a Session leaves the domain owing one; a browser Session opens only while a browser page can be admitted for it, else the request waits for a browser Session with room. `eager` opens a new Session for every request while the pool is under `session.pool.max_size`, as Crawlee does.",
    kind: ValueKind::Choice(GROWTHS),
    default: on_demand,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn one() -> Value {
    Value::from(1_u64)
}

/// `session.pool.session_concurrency` (spec #2139).
pub const SESSION_POOL_SESSION_CONCURRENCY: Setting = Setting {
    name: "session.pool.session_concurrency",
    description: "How many requests one Session of a crawl's Session pool serves at once under `session.pool.growth: on_demand`: 1. Unset, a Session serves any number, so the pool opens one Session for each client and proxy tier.",
    kind: ValueKind::Count,
    default: one,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The Sessions a crawl's Session pool prefers for a request (spec #2139).
pub const AFFINITIES: &[&str] = &["host", "none"];

fn host() -> Value {
    Value::from("host")
}

/// `session.pool.affinity` (spec #2139).
pub const SESSION_POOL_AFFINITY: Setting = Setting {
    name: "session.pool.affinity",
    description: "Which open Session with room a crawl's Session pool prefers for a request under `session.pool.growth: on_demand`. `host` prefers one whose last request went to the request's host; `none` draws among them at random. It never opens a Session.",
    kind: ValueKind::Choice(AFFINITIES),
    default: host,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn null() -> Value {
    Value::Null
}

fn ten_minutes() -> Value {
    Value::from(600_000_u64)
}

/// `escalation.proxy_kinds` (ADR 0021; spec #600).
pub const ESCALATION_PROXY_KINDS: Setting = Setting {
    name: "escalation.proxy_kinds",
    description: "The ordered proxy kinds (`datacenter`, `isp`, `residential`, `mobile`) a crawl's Session pool climbs on a 401, 403 or 429 without a challenge page: unset, the kinds the Run's Proxy provider configures, cheapest first, so a run with no paid pool configured spends nothing; set, it narrows that list. A Recipe may set it, as `proxy.kind` is; a Client acting for a model is refused with `setting_locked`.",
    kind: ValueKind::List(&ValueKind::Choice(PROXY_KINDS)),
    default: null,
    clearable: true,
    authority: Authority::NotByModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `escalation.trial_interval` (ADR 0021; spec #600).
pub const ESCALATION_TRIAL_INTERVAL: Setting = Setting {
    name: "escalation.trial_interval",
    description: "How often an escalated domain sends one trial request through a Session at the proxy kind below, stepping back down when it succeeds and keeping the costlier kind when it is blocked: 600 s (10 minutes). Unset (`null`), no periodic trial runs.",
    kind: ValueKind::Duration,
    default: ten_minutes,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
        SESSION_POOL_GROWTH,
        SESSION_POOL_SESSION_CONCURRENCY,
        SESSION_POOL_AFFINITY,
        ESCALATION_PROXY_KINDS,
        ESCALATION_TRIAL_INTERVAL,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
