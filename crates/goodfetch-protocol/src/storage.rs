//! The storage levels' contracts (ADR 0122, spec #213): the two settings of
//! the `storage.` area that set them, every key under `storage.` (ADR
//! 0083), and the level each filesystem a Run writes to is at, which the
//! Resource governor samples. One governor per engine process of either
//! Product, so everything here is Collie's.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Authority, Entries, ProtocolVersion, Setting, ValueKind};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A filesystem's storage level (ADR 0122), from the best to the worst.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StorageLevel {
    /// It has room: nothing is shed.
    Normal,
    /// Its free space or inodes fell below `storage.low_free`: the Runs
    /// writing to it shed what they can do without and carry
    /// `storage_low`.
    Low,
    /// Its free space or inodes fell below `storage.critical_free`: every
    /// Run with an essential write there stops resumably.
    Critical,
}

impl StorageLevel {
    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Low => "low",
            Self::Critical => "critical",
        }
    }
}

fn unset() -> serde_json::Value {
    serde_json::Value::Null
}

fn is_off(value: &serde_json::Value) -> bool {
    value.as_u64() == Some(0)
}

/// `storage.low_free` (ADR 0122).
pub const LOW_FREE: Setting = Setting {
    name: "storage.low_free",
    description: "The free space, in bytes, below which a filesystem a Run writes to is at the `low` storage level: its Runs go on, record no Run trace video, store no new Response cache entry, skip their periodic Checkpoints onto it, and carry the warning `storage_low`. Unset, each filesystem takes the smaller of 5 GiB and 10 % of its size, and free inodes go by 10 %; `0` turns the level off, which the Run's report lists in `weakened`. Read again at every sample of the Resource governor; set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: Some(is_off),
    arrived_in: ARRIVED_IN,
};

/// `storage.critical_free` (ADR 0122).
pub const CRITICAL_FREE: Setting = Setting {
    name: "storage.critical_free",
    description: "The free space, in bytes, below which a filesystem a Run writes to is at the `critical` storage level: every Run with an essential write on it stops resumably at its next safe point, with `resource: \"storage\"`, and a new Run whose run file would go there is refused with `resources_exhausted`. Unset, each filesystem takes the smaller of 1 GiB and 2 % of its size, and free inodes go by 2 %; `0` turns the level off, which the Run's report lists in `weakened`. Read again at every sample of the Resource governor; set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: Some(is_off),
    arrived_in: ARRIVED_IN,
};

/// Every setting of the storage levels, each under `storage.`.
pub const SETTINGS: [Setting; 2] = [LOW_FREE, CRITICAL_FREE];

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &SETTINGS,
    hook_stages: &[],
    shared_type_values: &[],
};
