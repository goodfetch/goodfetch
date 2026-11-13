//! The storage levels' contracts (ADR 0122, spec #213): the two settings of
//! the `storage.` area that set them, every key under `storage.` (ADR
//! 0083), and the level each filesystem a Run writes to is at, which the
//! Resource governor samples, with the storage metrics the ops listener
//! serves (ADR 0050). One governor per engine process of either Product,
//! so everything here is Collie's.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, Instrument, Metric, MetricLabel, ProtocolVersion, Setting, Stability,
    ValueKind,
};

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
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Normal, Self::Low, Self::Critical];

    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }

    /// Its value in `goodfetch.storage.level`: 0, 1 or 2.
    pub fn gauge(self) -> u64 {
        match self {
            Self::Normal => 0,
            Self::Low => 1,
            Self::Critical => 2,
        }
    }
}

/// What a folder the Resource governor samples holds, the `holds` label of
/// the storage metrics (ADR 0122). Foxhound adds `warc` later.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SampledFolder {
    /// The run folders: the run files and their Checkpoints.
    RunState,
    /// The Goodfetch home: the Usage ledger and the Politeness file.
    Home,
    /// A Run trace's folder.
    Trace,
    /// The engine's temporary folder.
    Temp,
}

impl SampledFolder {
    /// Every value, in declaration order.
    pub const ALL: [Self; 4] = [Self::RunState, Self::Home, Self::Trace, Self::Temp];

    /// Its value of the `holds` label.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// The `holds` label of the storage metrics: every [`SampledFolder`].
pub const HOLDS_LABEL: MetricLabel = MetricLabel {
    name: "holds",
    values: || crate::wire_name::all(&SampledFolder::ALL),
};

/// `goodfetch.storage.available` (ADR 0122): the space left on each
/// sampled filesystem.
pub const AVAILABLE: Metric = Metric {
    name: "goodfetch.storage.available",
    description: "The space left to the engine's user, in bytes, on each filesystem the Resource governor samples, at its last sample, labelled by what the filesystem holds; a filesystem holding several of them gives each its series (ADR 0122).",
    instrument: Instrument::Gauge,
    unit: "By",
    labels: &[HOLDS_LABEL],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.storage.inodes.available` (ADR 0122): the inodes left on
/// each sampled filesystem that reports them.
pub const INODES_AVAILABLE: Metric = Metric {
    name: "goodfetch.storage.inodes.available",
    description: "The inodes left to the engine's user on each filesystem the Resource governor samples that reports them, at its last sample, labelled by what the filesystem holds (ADR 0122).",
    instrument: Instrument::Gauge,
    unit: "{inode}",
    labels: &[HOLDS_LABEL],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.storage.level` (ADR 0122): each sampled filesystem's
/// storage level.
pub const LEVEL: Metric = Metric {
    name: "goodfetch.storage.level",
    description: "The storage level of each filesystem the Resource governor samples, at its last sample: 0 `normal`, 1 `low`, 2 `critical`, labelled by what the filesystem holds. `/readyz` fails while run state's is 2 (ADR 0122).",
    instrument: Instrument::Gauge,
    unit: "{level}",
    labels: &[HOLDS_LABEL],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

fn unset() -> serde_json::Value {
    serde_json::Value::Null
}

fn is_off(value: &serde_json::Value) -> bool {
    value.as_u64() == Some(0)
}

/// `storage.low_free` (ADR 0122).
pub const LOW_FREE: Setting = Setting {
    name: "storage.low_free",
    description: "The free space, in bytes, below which a filesystem a Run writes to is at the `low` storage level: its Runs go on, record no Run trace video, store no new Response cache entry, skip their periodic Checkpoints onto it, and carry the warning `storage_low`. Unset, each filesystem takes the smaller of 5 GiB and 10 % of its size, and free inodes go by 10 %; `0` turns the level off, which the Run's report lists in `weakened`. Each sample of the Resource governor reads it from the engine's settings: the engine's own write (`config.user_write`, `config.policy_write`) is seen at the next sample, a hand edit at the engine's next read of the file, by the next Run's start at the latest; set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: Some(is_off),
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `storage.critical_free` (ADR 0122).
pub const CRITICAL_FREE: Setting = Setting {
    name: "storage.critical_free",
    description: "The free space, in bytes, below which a filesystem a Run writes to is at the `critical` storage level: every Run with an essential write on it stops resumably at its next safe point, with `resource: \"storage\"`, and a new Run whose run file would go there is refused with `resources_exhausted`. Unset, each filesystem takes the smaller of 1 GiB and 2 % of its size, and free inodes go by 2 %; `0` turns the level off, which the Run's report lists in `weakened`. Each sample of the Resource governor reads it from the engine's settings: the engine's own write (`config.user_write`, `config.policy_write`) is seen at the next sample, a hand edit at the engine's next read of the file, by the next Run's start at the latest; set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: Some(is_off),
    fixed_for_run: false,
    unbuilt: None,
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
    metrics: &[AVAILABLE, INODES_AVAILABLE, LEVEL],
    settings: &SETTINGS,
    hook_stages: &[],
    shared_type_values: &[],
};
