//! The Completeness reasons every Run's end gives (ADRs 0029 and 0102):
//! each a value of a type both Products share, `completeness_reason_code`,
//! to which Foxhound adds a crawl's own. A reader keeps a code it does not
//! know, a newer engine's, as it is (G3).

use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Run stopped before its end.
pub const STOPPED: &str = "stopped";
/// An error ended the Run before its end.
pub const FAILED: &str = "failed";

/// The Completeness reason `name`, meaning `description`.
pub(crate) const fn reason(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::CompletenessReasonCode,
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
        reason(
            STOPPED,
            "It stopped before its end, `incomplete`; a crawl's counts the requests it left.",
        ),
        reason(FAILED, "An error ended it before its end, `incomplete`."),
    ],
};
