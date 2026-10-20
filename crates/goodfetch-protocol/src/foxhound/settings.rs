//! The keys of Foxhound's areas whose own modules do not exist yet, each with
//! its area first (ADR 0083): Paid fetches' (ADR 0045), which a Recipe or a
//! run only narrows or never sets, and the delivery's honesty guarantees,
//! which stay locked whatever the layer (ADR 0028). Each later area moves its
//! keys into its own module.

use serde_json::{Value, json};

use crate::catalog::{Authority, Entries, ProtocolVersion, Setting, ValueKind};

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
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
