//! The values Foxhound adds to the Usage ledger's shared types (ADRs 0041,
//! 0045, 0054 and 0088): the Usage kind `paid_fetch`, Paid fetches counted
//! by currency, whose Limits carry *must understand*, and the Limit scope of
//! a Partitioned crawl. A Collie engine knows them only by name: it refuses
//! them where they are set, keeps the rows holding them untouched and lists
//! them as Foxhound's.

use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Usage kind of Paid fetches, counted by currency (ADR 0045).
pub const PAID_FETCH: &str = "paid_fetch";

/// The Limit scope kind of a Partitioned crawl (ADR 0054).
pub const PARTITIONED_CRAWL: &str = "partitioned_crawl";

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: PAID_FETCH,
            description: "Paid fetches and their amounts, by currency (ADR 0045); its Limits carry must understand.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: PARTITIONED_CRAWL,
            description: "Every Partition of a Partitioned crawl (ADR 0054).",
            arrived_in: ARRIVED_IN,
        },
    ],
};
