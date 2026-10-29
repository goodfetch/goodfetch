//! The keys of Foxhound's areas whose own modules do not exist yet, each with
//! its area first (ADR 0083): Paid fetches' (ADR 0045), which a Recipe or a
//! run only narrows or never sets, the delivery's honesty guarantees, which
//! stay locked whatever the layer (ADR 0028), and a crawl's per-domain
//! Politeness (ADRs 0053 and 0086), shared by every run of the user. Each
//! later area moves its keys into its own module.

use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ProtocolVersion, Setting, SharedType, SharedTypeValue, ValueKind,
};
use crate::politeness::POLITENESS_SHARING;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn null() -> Value {
    Value::Null
}

fn empty_list() -> Value {
    json!([])
}

fn empty_table() -> Value {
    json!({})
}

fn yes() -> Value {
    Value::Bool(true)
}

fn two_seconds() -> Value {
    json!(2_000)
}

fn a_minute() -> Value {
    json!(60_000)
}

fn fifteen_minutes() -> Value {
    json!(900_000)
}

fn thirty_seconds() -> Value {
    json!(30_000)
}

/// A value below `default` milliseconds, which waits less than the built-in
/// default and so presses a site harder (ADR 0086).
fn shorter_than(value: &Value, default: u64) -> bool {
    value
        .as_u64()
        .is_some_and(|milliseconds| milliseconds < default)
}

fn switched_off(value: &Value) -> bool {
    value == &Value::Bool(false)
}

fn initial_shortened(value: &Value) -> bool {
    shorter_than(value, 2_000)
}

fn max_shortened(value: &Value) -> bool {
    shorter_than(value, 60_000)
}

/// More than one request in flight on a domain key, ADR 0086's built-in
/// limit.
fn more_than_one(value: &Value) -> bool {
    value.as_u64().is_some_and(|concurrency| concurrency > 1)
}

/// Less than a second between two starts on a domain key, ADR 0086's
/// built-in delay.
fn under_a_second(value: &Value) -> bool {
    shorter_than(value, 1_000)
}

/// The Audit log kind of a Politeness or Site-preferences value that
/// presses a site harder than its built-in default (ADR 0086), which
/// Foxhound's composition records once per run per key.
pub const POLITENESS_WEAKENED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "politeness_weakened",
    description: "A run set a Politeness or Site-preferences value that presses a site harder than its built-in default, or a `site_preferences.robots_txt_user_agent`, once per run per key, with the key, its value, its built-in default and the layer that set it (ADR 0086).",
    arrived_in: ARRIVED_IN,
};

/// A Politeness or Site-preferences key whose pressing-harder values
/// Foxhound's composition records as [`POLITENESS_WEAKENED`], with the
/// built-in default ADR 0086 measures them against.
#[derive(Clone, Copy)]
pub struct PressingHarder {
    /// The key.
    pub setting: &'static Setting,
    /// ADR 0086's built-in default: the catalog's, but for the per-domain
    /// concurrency and delay, which hold none until the fetch tiers set 1
    /// and 1 s.
    pub built_in: fn() -> Value,
}

fn one_second() -> Value {
    json!(1_000)
}

/// The Politeness and Site-preferences keys whose pressing-harder values
/// Foxhound's composition records as [`POLITENESS_WEAKENED`] (ADR 0086).
pub const PRESSING_HARDER: [PressingHarder; 8] = [
    PressingHarder {
        setting: &POLITENESS_DOMAIN_CONCURRENCY,
        built_in: one,
    },
    // Anything but `shared` presses harder (ADR 0086), against `shared`.
    PressingHarder {
        setting: &POLITENESS_SHARING,
        built_in: POLITENESS_SHARING.default,
    },
    PressingHarder {
        setting: &POLITENESS_DOMAIN_DELAY,
        built_in: one_second,
    },
    PressingHarder {
        setting: &POLITENESS_BACKOFF_ENABLED,
        built_in: POLITENESS_BACKOFF_ENABLED.default,
    },
    PressingHarder {
        setting: &POLITENESS_BACKOFF_INITIAL,
        built_in: POLITENESS_BACKOFF_INITIAL.default,
    },
    PressingHarder {
        setting: &POLITENESS_BACKOFF_MAX,
        built_in: POLITENESS_BACKOFF_MAX.default,
    },
    PressingHarder {
        setting: &SITE_PREFERENCES_RESPECT_ROBOTS_TXT,
        built_in: SITE_PREFERENCES_RESPECT_ROBOTS_TXT.default,
    },
    PressingHarder {
        setting: &SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
        built_in: SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.default,
    },
];

/// `paid_fetch.domains` (ADR 0045).
pub const PAID_FETCH_DOMAINS: Setting = Setting {
    name: "paid_fetch.domains",
    description: "The registrable domains a run may pay for a page; `*` only when written. Widened only in an Operator policy, user or project configuration, or by a person's answer; a Recipe or a run only narrows it.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Narrowed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `paid_fetch.max_price_per_page` (ADR 0045).
pub const PAID_FETCH_MAX_PRICE_PER_PAGE: Setting = Setting {
    name: "paid_fetch.max_price_per_page",
    description: "The most a run pays for one page, as an amount and its currency such as `0.05 USD`; none by default. Raised only in configuration; a Recipe or a run only lowers it.",
    kind: ValueKind::Price,
    default: null,
    clearable: true,
    authority: Authority::Narrowed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `paid_fetch.exchange_rates` (ADR 0045).
pub const PAID_FETCH_EXCHANGE_RATES: Setting = Setting {
    name: "paid_fetch.exchange_rates",
    description: "The fixed rates that convert a price into a Limit's currency, by currency pair. Set only in an Operator policy and user and project configuration.",
    kind: ValueKind::Map(&ValueKind::Text),
    default: empty_table,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `paid_fetch.chain_rpc_endpoints` (ADR 0045).
pub const PAID_FETCH_CHAIN_RPC_ENDPOINTS: Setting = Setting {
    name: "paid_fetch.chain_rpc_endpoints",
    description: "The RPC endpoint of each chain whose finalized state proves a payment did not settle, by chain. Set only in an Operator policy and user and project configuration.",
    kind: ValueKind::Map(&ValueKind::Text),
    default: empty_table,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `delivery.write_ids_and_epochs` (ADR 0028).
pub const DELIVERY_WRITE_IDS_AND_EPOCHS: Setting = Setting {
    name: "delivery.write_ids_and_epochs",
    description: "Every write a Sink makes carries its delivery ID and run epoch, an honesty guarantee that stays locked whatever the layer.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Fixed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `delivery.report_guarantee_class` (ADR 0028).
pub const DELIVERY_REPORT_GUARANTEE_CLASS: Setting = Setting {
    name: "delivery.report_guarantee_class",
    description: "Every delivery reports its Guarantee class, an honesty guarantee that stays locked whatever the layer.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Fixed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `politeness.domain.concurrency` (ADRs 0053 and 0086).
pub const POLITENESS_DOMAIN_CONCURRENCY: Setting = Setting {
    name: "politeness.domain.concurrency",
    description: "How many requests may be in flight at once on one domain key, counted across every run of the user that shares Politeness: a run admits one only while that combined count is below its own value. None by default until the fetch tiers set ADR 0086's default of 1. A value above 1 is listed in the run's result as one that presses a site harder.",
    kind: ValueKind::Count,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: Some(more_than_one),
    arrived_in: ARRIVED_IN,
};

/// `politeness.domain.delay` (ADRs 0053 and 0086).
pub const POLITENESS_DOMAIN_DELAY: Setting = Setting {
    name: "politeness.domain.delay",
    description: "How long a run waits after the last request start on a domain key, by any run of the user that shares Politeness, before it starts another there. None by default until the fetch tiers set ADR 0086's default of 1 s. A value below 1 s is listed in the run's result as one that presses a site harder.",
    kind: ValueKind::Duration,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: Some(under_a_second),
    arrived_in: ARRIVED_IN,
};

fn no() -> Value {
    Value::Bool(false)
}

fn one() -> Value {
    json!(1)
}

/// `politeness.domain.adaptive_delay` (spec #582's Settings; ADR 0086).
pub const POLITENESS_DOMAIN_ADAPTIVE_DELAY: Setting = Setting {
    name: "politeness.domain.adaptive_delay",
    description: "Whether a domain key's delay between request starts lengthens as its responses slow: on, the delay is the longer of `politeness.domain.delay` and the exponentially weighted mean (weight 0.3) of the latency of its last responses on the `http` tier, that mean capped at `politeness.backoff.max`. It only lengthens the delay, never below `politeness.domain.delay`. Off by default.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `politeness.domain.rate` (spec #582's Settings; ADRs 0053 and 0086).
pub const POLITENESS_DOMAIN_RATE: Setting = Setting {
    name: "politeness.domain.rate",
    description: "How many crawl requests a minute may start on one domain key, counted across every run of the user that shares Politeness, as a token bucket of `politeness.domain.burst` tokens refilled at this rate: 30 holds a site to 30 requests a minute. Each run is held to its own value; the delay between starts still applies. None by default.",
    kind: ValueKind::Count,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `politeness.domain.burst` (spec #582's Settings; ADRs 0053 and 0086).
pub const POLITENESS_DOMAIN_BURST: Setting = Setting {
    name: "politeness.domain.burst",
    description: "How many crawl requests may start at once on one domain key under `politeness.domain.rate`, the size of its token bucket: 1 by default, so the starts are spread evenly over the minute. A value of 0 is read as 1.",
    kind: ValueKind::Count,
    default: one,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `politeness.backoff.enabled` (ADRs 0021, 0023 and 0086).
pub const POLITENESS_BACKOFF_ENABLED: Setting = Setting {
    name: "politeness.backoff.enabled",
    description: "Whether a domain that answers 429, 502, 503 or 504, or does not answer in time, is backed off, and its `Retry-After` waited out, for every run of the user that shares Politeness. Off, the run ignores both, and it is listed in the run's result.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(switched_off),
    arrived_in: ARRIVED_IN,
};

/// `politeness.backoff.initial` (ADRs 0021, 0086 and 0118).
pub const POLITENESS_BACKOFF_INITIAL: Setting = Setting {
    name: "politeness.backoff.initial",
    description: "The shortest wait of a domain's backoff: each wait is drawn at random between it and three times the previous wait (the first between it and three times it), never above `politeness.backoff.max`: 2 s. A shorter value is listed in the run's result.",
    kind: ValueKind::Duration,
    default: two_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(initial_shortened),
    arrived_in: ARRIVED_IN,
};

/// `politeness.backoff.max` (ADRs 0021, 0086 and 0118).
pub const POLITENESS_BACKOFF_MAX: Setting = Setting {
    name: "politeness.backoff.max",
    description: "The longest wait of a domain's backoff: 60 s. A `Retry-After` may hold the domain longer, up to `politeness.retry_after.max`. A shorter value is listed in the run's result.",
    kind: ValueKind::Duration,
    default: a_minute,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(max_shortened),
    arrived_in: ARRIVED_IN,
};

/// `politeness.retry_after.max` (ADR 0118).
pub const POLITENESS_RETRY_AFTER_MAX: Setting = Setting {
    name: "politeness.retry_after.max",
    description: "The longest a crawl waits for the time a site's `Retry-After` asks, past `politeness.backoff.max`: 15 minutes. When the site asks for longer, nothing is sent to its domain before that time and nothing waits for it: the request whose response asked, and each request admitted to the domain meanwhile, ends `failed` with `response_unavailable`, and `run.retry_failed` re-admits them once the site is back. Whatever its value, nothing is sent sooner than the site asked.",
    kind: ValueKind::Duration,
    default: fifteen_minutes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// Any value, which names a robots.txt group beside Foxhound's and so may
/// press a site harder (ADR 0086).
fn set(value: &Value) -> bool {
    !value.is_null()
}

/// `site_preferences.robots_txt_user_agent` (ADRs 0086 and 0090).
pub const SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT: Setting = Setting {
    name: "site_preferences.robots_txt_user_agent",
    description: "The operator's own robots.txt product token, letters, `_` and `-`: a crawl obeys the robots.txt groups naming it merged with those naming `goodfetch-foxhound`, each matched as a whole token in any case, and the `*` group only when neither has one. None by default. A value is listed in the run's result as one that presses a site harder.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: Some(set),
    arrived_in: ARRIVED_IN,
};

/// `site_preferences.respect_robots_txt` (ADRs 0016 and 0086).
pub const SITE_PREFERENCES_RESPECT_ROBOTS_TXT: Setting = Setting {
    name: "site_preferences.respect_robots_txt",
    description: "Whether every crawl request, start URLs and sitemap entries included, and every navigation a handler makes, obeys the site's robots.txt (RFC 9309): a request it disallows, or one waiting on a robots.txt that answers 429 or 5xx or no answer, with no earlier copy, until its retries ran out, ends `skipped` with `robots_txt` and its handler never runs. On by default. Off, robots.txt is read but not obeyed, and it is listed in the run's result as a value that presses a site harder.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(switched_off),
    arrived_in: ARRIVED_IN,
};

/// `site_preferences.max_crawl_delay` (ADRs 0016 and 0086).
pub const SITE_PREFERENCES_MAX_CRAWL_DELAY: Setting = Setting {
    name: "site_preferences.max_crawl_delay",
    description: "The longest robots.txt Crawl-delay a crawl waits out between request starts on a domain key while `site_preferences.respect_robots_txt` is on: 30 s. A host asking for more has its crawl requests skipped with `crawl_delay`, and its delay is not raised.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
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
    settings: &[
        PAID_FETCH_DOMAINS,
        PAID_FETCH_MAX_PRICE_PER_PAGE,
        PAID_FETCH_EXCHANGE_RATES,
        PAID_FETCH_CHAIN_RPC_ENDPOINTS,
        DELIVERY_WRITE_IDS_AND_EPOCHS,
        DELIVERY_REPORT_GUARANTEE_CLASS,
        POLITENESS_DOMAIN_CONCURRENCY,
        POLITENESS_DOMAIN_DELAY,
        POLITENESS_DOMAIN_ADAPTIVE_DELAY,
        POLITENESS_DOMAIN_RATE,
        POLITENESS_DOMAIN_BURST,
        POLITENESS_BACKOFF_ENABLED,
        POLITENESS_BACKOFF_INITIAL,
        POLITENESS_BACKOFF_MAX,
        POLITENESS_RETRY_AFTER_MAX,
        SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
        SITE_PREFERENCES_RESPECT_ROBOTS_TXT,
        SITE_PREFERENCES_MAX_CRAWL_DELAY,
    ],
    hook_stages: &[],
    shared_type_values: &[POLITENESS_WEAKENED],
};
