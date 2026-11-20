//! How a Run's state reaches its run file (ADRs 0002, 0138 and 0142): the
//! Durability level, each value a Preset of the Run-state batch keys
//! (`run_state.batch.*` and `run_state.commit_interval`), which a layer may
//! also set one by one, the threshold a batch key takes, a count or
//! `none`, and the memory budget past which written state leaves memory.

use std::num::NonZeroU64;

use serde_json::Value;

use crate::catalog::{Authority, ProtocolVersion, Setting, ValueKind, ValueShape};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `run_state.durability`'s default: the Run's changes in Run-state
/// batches, each synced (ADR 0142).
pub const DURABILITY_BATCH: &str = "batch";
/// `run_state.durability`'s opt-in: each attempt's claim and outcome a
/// synced batch of its own, so a power cut loses only the attempts in
/// flight (ADRs 0002, 0138 and 0142).
pub const DURABILITY_FULL: &str = "full";
/// `run_state.durability`'s opt-in: as `full`, never synced, so an
/// acknowledged commit survives a process crash only (ADRs 0002 and 0138).
pub const DURABILITY_FAST: &str = "fast";
/// `run_state.durability`'s opt-in: as `fast`, with one synced batch every
/// `run_state.commit_interval` (ADR 0138).
pub const DURABILITY_PERIODIC: &str = "periodic";
/// `run_state.durability`'s opt-in: as `batch`, nothing synced, and a run
/// file a crash left is discarded, so the Run starts again (ADRs 0138 and
/// 0142).
pub const DURABILITY_OFF: &str = "off";

/// `run_state.batch.sync`'s default: every batch synced before the next
/// starts.
pub const SYNC_EACH: &str = "each";
/// `run_state.batch.sync`: one synced batch per `run_state.commit_interval`.
pub const SYNC_INTERVAL: &str = "interval";
/// `run_state.batch.sync`: no batch synced; a process crash loses nothing
/// written.
pub const SYNC_NEVER: &str = "never";
/// `run_state.batch.sync`: nothing synced, and a crash discards the run
/// file.
pub const SYNC_OFF: &str = "off";

/// The threshold value that never seals a batch.
pub const NO_THRESHOLD: &str = "none";

/// `run_state.batch.pages`' default.
pub const BATCH_PAGES: u64 = 1_000;
/// `run_state.batch.interval`'s default, in milliseconds.
pub const BATCH_INTERVAL_MS: u64 = 1_000;
/// `run_state.batch.bytes`' default: 8 MiB.
pub const BATCH_BYTES: u64 = 8 * 1024 * 1024;
/// `run_state.commit_interval`'s default, in milliseconds.
pub const COMMIT_INTERVAL_MS: u64 = 5_000;
/// `run_state.memory.budget`'s default: 64 MiB.
pub const MEMORY_BUDGET: u64 = 64 * 1024 * 1024;

fn batch_durability() -> Value {
    Value::String(DURABILITY_BATCH.to_owned())
}

/// Only `off` weakens on its own: the keys of its preset say the rest, a
/// `sync` other than `each` covering `fast` and `periodic` (ADR 0142).
fn durability_off(value: &Value) -> bool {
    value.as_str() == Some(DURABILITY_OFF)
}

/// `run_state.durability` (ADRs 0002, 0138 and 0142).
pub const RUN_STATE_DURABILITY: Setting = Setting {
    name: "run_state.durability",
    description: "How a Run's state reaches the disk, its Durability level, each value a preset of `run_state.batch.*` and `run_state.commit_interval`: `batch`, the default, writes the Run's changes in Run-state batches, of 1,000 attempts, 1 s or 8 MiB, whichever comes first, each synced, so a crash or a power cut loses at most the last batch; `full` writes each attempt's claim and outcome as a synced batch of its own (`synchronous=FULL`, with `fullfsync` on macOS), so only the attempts in flight are lost; `fast` is `full` unsynced (`synchronous=NORMAL`): it survives a process crash only, since an OS crash or a power cut may roll back recent commits, possibly many, without damaging the run state; `periodic` is `fast` with one synced batch every `run_state.commit_interval`, so a power cut loses at most that interval of work; `off` writes as `batch` does and syncs nothing (`synchronous=OFF`), and a run file a crash or a power cut left is discarded when the Run is next started or resumed, so the Run starts again from nothing. A level unfolds into its preset's keys at the layer that set it: a key set at that layer or a higher one overrides the preset's value, and a lock on the level locks every key of its preset. At every level a clean stop writes and syncs everything, so a paused Run resumes. The run report's `weakened` names `off`, and each preset key whose value weakens the Run.",
    kind: ValueKind::Choice(&[
        DURABILITY_FULL,
        DURABILITY_FAST,
        DURABILITY_PERIODIC,
        DURABILITY_BATCH,
        DURABILITY_OFF,
    ]),
    default: batch_durability,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(durability_off),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn no() -> Value {
    Value::Bool(false)
}

fn yes() -> Value {
    Value::Bool(true)
}

/// `run_state.batch.per_attempt` (ADR 0142).
pub const RUN_STATE_BATCH_PER_ATTEMPT: Setting = Setting {
    name: "run_state.batch.per_attempt",
    description: "Write each attempt's claim before its fetch and its outcome before anything is told outside, as its own batch. Slowest, and a crash then loses only the attempts in flight.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A threshold of a Run-state batch: a count of at least 1, or `none`,
/// which never seals one (ADR 0142).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchThreshold {
    /// A batch is sealed once this much is reached.
    At(NonZeroU64),
    /// `none`: no batch is sealed by this threshold.
    Never,
}

impl BatchThreshold {
    /// The threshold `value` gives.
    ///
    /// # Errors
    ///
    /// When `value` is neither a whole number of at least 1 nor `none`.
    pub fn read(value: &Value) -> Result<Self, String> {
        if value.as_str() == Some(NO_THRESHOLD) {
            return Ok(Self::Never);
        }
        value
            .as_u64()
            .and_then(NonZeroU64::new)
            .map(Self::At)
            .ok_or_else(|| {
                format!("expected a whole number of at least 1, or `none`, found `{value}`")
            })
    }

    /// Whether it seals later than `default`, or never.
    pub fn above(self, default: u64) -> bool {
        match self {
            Self::At(at) => at.get() > default,
            Self::Never => true,
        }
    }
}

/// The value [`BatchThreshold::read`] reads it back from.
impl serde::Serialize for BatchThreshold {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::At(at) => serializer.serialize_u64(at.get()),
            Self::Never => serializer.serialize_str(NO_THRESHOLD),
        }
    }
}

/// A value of `run_state.batch.pages`, `.interval` or `.bytes`.
pub const BATCH_THRESHOLD_SHAPE: ValueShape = ValueShape {
    name: "BatchThreshold",
    check: |value| BatchThreshold::read(value).map(drop),
    schema: || {
        serde_json::json!({
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Attempts, milliseconds or bytes, by the key, at which a batch is sealed.",
                },
                {
                    "const": NO_THRESHOLD,
                    "description": "No batch is sealed by this threshold.",
                },
            ],
        })
    },
    examples: || {
        vec![
            serde_json::json!(BATCH_PAGES),
            serde_json::json!(NO_THRESHOLD),
        ]
    },
};

/// Whether `value`, a threshold, seals later than `default` or never.
fn threshold_above(value: &Value, default: u64) -> bool {
    BatchThreshold::read(value).is_ok_and(|threshold| threshold.above(default))
}

fn batch_pages() -> Value {
    serde_json::json!(BATCH_PAGES)
}

fn pages_above(value: &Value) -> bool {
    threshold_above(value, BATCH_PAGES)
}

/// `run_state.batch.pages` (ADR 0142).
pub const RUN_STATE_BATCH_PAGES: Setting = Setting {
    name: "run_state.batch.pages",
    description: "Write the Run's changes once this many attempts have ended since the last batch. Lower loses less to a crash and writes more often.",
    kind: ValueKind::Shape(&BATCH_THRESHOLD_SHAPE),
    default: batch_pages,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(pages_above),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn batch_interval() -> Value {
    serde_json::json!(BATCH_INTERVAL_MS)
}

fn interval_above(value: &Value) -> bool {
    threshold_above(value, BATCH_INTERVAL_MS)
}

/// `run_state.batch.interval` (ADR 0142).
pub const RUN_STATE_BATCH_INTERVAL: Setting = Setting {
    name: "run_state.batch.interval",
    description: "Write the Run's changes at least this often. A crash loses at most about this much work.",
    kind: ValueKind::Shape(&BATCH_THRESHOLD_SHAPE),
    default: batch_interval,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(interval_above),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn batch_bytes() -> Value {
    serde_json::json!(BATCH_BYTES)
}

fn bytes_above(value: &Value) -> bool {
    threshold_above(value, BATCH_BYTES)
}

/// `run_state.batch.bytes` (ADR 0142).
pub const RUN_STATE_BATCH_BYTES: Setting = Setting {
    name: "run_state.batch.bytes",
    description: "Write the Run's changes once they hold this many bytes of rows. This bounds the memory that changes not yet written take.",
    kind: ValueKind::Shape(&BATCH_THRESHOLD_SHAPE),
    default: batch_bytes,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(bytes_above),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn sync_each() -> Value {
    Value::String(SYNC_EACH.to_owned())
}

/// Every `sync` but `each` lets a power cut take a written batch back.
fn sync_below_each(value: &Value) -> bool {
    value.as_str() != Some(SYNC_EACH)
}

/// `run_state.batch.sync` (ADR 0142).
pub const RUN_STATE_BATCH_SYNC: Setting = Setting {
    name: "run_state.batch.sync",
    description: "Which batches reach the disk before the next one starts. `each` survives a power cut; `interval` syncs one batch per `run_state.commit_interval`; `never` survives a process crash only; `off` syncs nothing, so a crash discards the run file and the Run starts again.",
    kind: ValueKind::Choice(&[SYNC_EACH, SYNC_INTERVAL, SYNC_NEVER, SYNC_OFF]),
    default: sync_each,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(sync_below_each),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn five_seconds() -> Value {
    serde_json::json!(COMMIT_INTERVAL_MS)
}

/// `run_state.commit_interval` (ADRs 0138 and 0142).
pub const RUN_STATE_COMMIT_INTERVAL: Setting = Setting {
    name: "run_state.commit_interval",
    description: "How often `run_state.batch.sync: interval` syncs a batch, and an idle Run's last changes.",
    kind: ValueKind::Duration,
    default: five_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn memory_budget() -> Value {
    serde_json::json!(MEMORY_BUDGET)
}

/// `run_state.memory.budget` (ADR 0142, spec #2138).
pub const RUN_STATE_MEMORY_BUDGET: Setting = Setting {
    name: "run_state.memory.budget",
    description: "The most memory this Run's Frontier, Leases and changes not yet written may hold, in bytes. Past it, or under memory pressure, state already written leaves memory and is read back from the run file. 0 keeps the Frontier in the run file. A budget of 0 with `run_state.batch.pages`, `.interval` and `.bytes` all `none` is refused, since nothing would ever leave memory.",
    kind: ValueKind::Count,
    default: memory_budget,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// One key of a Durability level's preset and the value it gives.
#[derive(Clone, Copy)]
pub struct PresetValue {
    /// The key.
    pub setting: &'static Setting,
    /// Its value under the level.
    pub value: fn() -> Value,
}

const fn preset(setting: &'static Setting, value: fn() -> Value) -> PresetValue {
    PresetValue { setting, value }
}

fn sync_interval() -> Value {
    Value::String(SYNC_INTERVAL.to_owned())
}

fn sync_never() -> Value {
    Value::String(SYNC_NEVER.to_owned())
}

fn sync_off() -> Value {
    Value::String(SYNC_OFF.to_owned())
}

const FULL_PRESET: &[PresetValue] = &[
    preset(&RUN_STATE_BATCH_PER_ATTEMPT, yes),
    preset(&RUN_STATE_BATCH_SYNC, sync_each),
];

const FAST_PRESET: &[PresetValue] = &[
    preset(&RUN_STATE_BATCH_PER_ATTEMPT, yes),
    preset(&RUN_STATE_BATCH_SYNC, sync_never),
];

const PERIODIC_PRESET: &[PresetValue] = &[
    preset(&RUN_STATE_BATCH_PER_ATTEMPT, yes),
    preset(&RUN_STATE_BATCH_SYNC, sync_interval),
    preset(&RUN_STATE_COMMIT_INTERVAL, five_seconds),
];

const BATCH_PRESET: &[PresetValue] = &[
    preset(&RUN_STATE_BATCH_PER_ATTEMPT, no),
    preset(&RUN_STATE_BATCH_PAGES, batch_pages),
    preset(&RUN_STATE_BATCH_INTERVAL, batch_interval),
    preset(&RUN_STATE_BATCH_BYTES, batch_bytes),
    preset(&RUN_STATE_BATCH_SYNC, sync_each),
];

const OFF_PRESET: &[PresetValue] = &[
    preset(&RUN_STATE_BATCH_PER_ATTEMPT, no),
    preset(&RUN_STATE_BATCH_PAGES, batch_pages),
    preset(&RUN_STATE_BATCH_INTERVAL, batch_interval),
    preset(&RUN_STATE_BATCH_BYTES, batch_bytes),
    preset(&RUN_STATE_BATCH_SYNC, sync_off),
];

/// The keys the Durability level `level` sets, each with its value, as
/// ADR 0142's table gives them: a key a level leaves unused, such as a
/// threshold under `per_attempt`, it does not set. Empty for a value that
/// names no level, which the settings resolver never lets through.
pub fn durability_preset(level: &str) -> &'static [PresetValue] {
    match level {
        DURABILITY_FULL => FULL_PRESET,
        DURABILITY_FAST => FAST_PRESET,
        DURABILITY_PERIODIC => PERIODIC_PRESET,
        DURABILITY_BATCH => BATCH_PRESET,
        DURABILITY_OFF => OFF_PRESET,
        _ => &[],
    }
}

/// Every key some level's preset sets.
pub const PRESET_KEYS: &[&Setting] = &[
    &RUN_STATE_BATCH_PER_ATTEMPT,
    &RUN_STATE_BATCH_PAGES,
    &RUN_STATE_BATCH_INTERVAL,
    &RUN_STATE_BATCH_BYTES,
    &RUN_STATE_BATCH_SYNC,
    &RUN_STATE_COMMIT_INTERVAL,
];
