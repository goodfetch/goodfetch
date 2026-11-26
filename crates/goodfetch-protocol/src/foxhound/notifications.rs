//! The Notification events Foxhound adds (ADRs 0027, 0036 and 0095): a
//! Run's Records changed against its Baseline, a Sink behind, failed,
//! paused or recovered, and a Dataset's schema changed. A Collie engine
//! knows them only by name: it refuses them in a channel's `events` with
//! `not_in_product` and lists them as Foxhound's.

use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// One of Foxhound's events in the catalog's open value list.
const fn event(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::NotificationEvent,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

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
        event(
            "records_changed",
            "A Run's Records changed against its Baseline.",
        ),
        event("sink_behind", "A Sink fell behind its Run."),
        event("sink_failed", "A Sink failed."),
        event("sink_paused", "A Sink was paused."),
        event("sink_recovered", "A Sink recovered."),
        event("dataset_schema_changed", "A Dataset's schema changed."),
    ],
};
