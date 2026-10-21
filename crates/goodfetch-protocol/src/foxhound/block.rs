//! The Block cause only Foxhound carries (ADR 0095): `decoy`, which its
//! Frontier gives a page reached through a trap.

use crate::block::BlockCause;
use crate::catalog::Entries;

/// Foxhound's Block cause, `decoy`.
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
    shared_type_values: &[BlockCause::Decoy.shared_type_value()],
};
